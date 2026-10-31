//! Series hydration through SMG's title surface (`titles` by SMG title id).
//! SMG always serves the title surface, so a series it cannot answer is a
//! clear hydration failure.

use super::*;

fn season(number: i32, tvdb_id: i64, tmdb_id: Option<i64>) -> SeasonMetadata {
    SeasonMetadata {
        tvdb_id,
        tmdb_id,
        number,
        label: format!("Season {number}"),
        episode_type: "default".to_string(),
    }
}

fn episode(
    season_number: i32,
    episode_number: i32,
    tvdb_id: i64,
    tmdb_id: Option<i64>,
) -> EpisodeMetadata {
    EpisodeMetadata {
        tvdb_id,
        tmdb_id,
        episode_number,
        name: format!("Episode {season_number}x{episode_number}"),
        aired: "2026-01-01".to_string(),
        runtime_minutes: 30,
        is_filler: false,
        is_recap: false,
        overview: String::new(),
        absolute_number: String::new(),
        contiguous_absolute_number: None,
        season_number,
        image_url: String::new(),
    }
}

/// A series TMDB owns: SMG serves it with `tvdb_id: null`.
fn tmdb_primary_series(smg_id: i64, tmdb_id: i64, name: &str) -> SeriesMetadata {
    SeriesMetadata {
        smg_id: Some(smg_id),
        primary_source: "tmdb".to_string(),
        tvdb_id: 0,
        tmdb_id: Some(tmdb_id),
        name: name.to_string(),
        sort_name: name.to_string(),
        slug: name.to_ascii_lowercase().replace(' ', "-"),
        year: Some(2026),
        content_status: "continuing".to_string(),
        first_aired: "2026-01-01".to_string(),
        seasons: vec![season(1, 0, Some(tmdb_id * 10 + 1))],
        episodes: vec![
            episode(1, 1, 0, Some(tmdb_id * 100 + 1)),
            episode(1, 2, 0, Some(tmdb_id * 100 + 2)),
        ],
        ..Default::default()
    }
}

/// A TVDB-backed series, as either surface serves it.
fn tvdb_series(tvdb_id: i64, smg_id: Option<i64>, name: &str) -> SeriesMetadata {
    SeriesMetadata {
        smg_id,
        primary_source: "tvdb".to_string(),
        tvdb_id,
        name: name.to_string(),
        sort_name: name.to_string(),
        slug: name.to_ascii_lowercase().replace(' ', "-"),
        year: Some(2026),
        content_status: "continuing".to_string(),
        first_aired: "2026-01-01".to_string(),
        seasons: vec![season(1, tvdb_id * 10 + 1, None)],
        episodes: vec![
            episode(1, 1, tvdb_id * 100 + 1, None),
            episode(1, 2, tvdb_id * 100 + 2, None),
        ],
        ..Default::default()
    }
}

#[derive(Default)]
struct SeriesTitleGateway {
    /// Answer `get_series_titles` with a gateway error.
    failing: bool,
    /// Series the title surface serves.
    titles: Vec<SeriesMetadata>,
    redirects: Vec<(i64, i64)>,
    title_calls: Mutex<Vec<(Vec<SeriesTitleRef>, bool, bool)>>,
    /// Series TVDB ids asked of the TVDB-keyed `metadataBulk` document.
    /// Series hydration never uses it, so every test expects this to stay empty.
    metadata_bulk_calls: Mutex<Vec<i64>>,
}

#[async_trait]
impl MetadataGateway for SeriesTitleGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Err(AppError::Repository(
            "not used by series hydration tests".into(),
        ))
    }

    async fn search_tvdb_batch(
        &self,
        _queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        Err(AppError::Repository(
            "not used by series hydration tests".into(),
        ))
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        Err(AppError::Repository(
            "not used by series hydration tests".into(),
        ))
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Err(AppError::Repository(
            "not used by series hydration tests".into(),
        ))
    }

    async fn get_movie(&self, tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        Err(AppError::NotFound(format!("movie {tvdb_id}")))
    }

    async fn get_metadata_bulk(
        &self,
        _movie_tvdb_ids: &[i64],
        series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        self.metadata_bulk_calls
            .lock()
            .await
            .extend(series_tvdb_ids.iter().copied());
        Ok(BulkMetadataResult::default())
    }

    async fn get_series_titles(
        &self,
        refs: &[SeriesTitleRef],
        _language: &str,
        include_episodes: bool,
        include_episode_orders: bool,
    ) -> AppResult<SeriesTitleBulkResult> {
        self.title_calls.lock().await.push((
            refs.to_vec(),
            include_episodes,
            include_episode_orders,
        ));
        if self.failing {
            return Err(AppError::Repository("fixture title surface failure".into()));
        }
        let mut result = SeriesTitleBulkResult {
            redirects: self.redirects.clone(),
            ..Default::default()
        };
        for (ref_index, series_ref) in refs.iter().enumerate() {
            let target_smg_id = series_ref.smg_id.map(|smg_id| {
                self.redirects
                    .iter()
                    .find(|(from, _)| *from == smg_id)
                    .map_or(smg_id, |(_, to)| *to)
            });
            let series = self.titles.iter().find(|series| {
                target_smg_id.is_some_and(|smg_id| series.smg_id == Some(smg_id))
                    || series_ref
                        .tvdb_id
                        .is_some_and(|tvdb_id| series.tvdb_id == tvdb_id)
                    || series_ref
                        .tmdb_id
                        .is_some_and(|tmdb_id| series.tmdb_id == Some(tmdb_id))
            });
            match series {
                Some(series) => {
                    result.by_ref_index.insert(ref_index, series.clone());
                }
                None => result.missing_ref_indexes.push(ref_index),
            }
        }
        Ok(result)
    }
}

fn series_title(name: &str, external_ids: Vec<ExternalId>) -> NewTitle {
    NewTitle {
        name: name.to_string(),
        facet: MediaFacet::Series,
        monitored: true,
        tags: vec![],
        external_ids,
        min_availability: None,
        ..Default::default()
    }
}

fn target(title: &Title) -> crate::catalog_workflow::HydrationTarget {
    crate::catalog_workflow::HydrationTarget {
        title: title.clone(),
        requested_tvdb_id: None,
        requested_movie_ref: None,
        sync_wanted_after_completion: false,
        source: crate::catalog_workflow::HydrationSource::Interactive,
    }
}

fn external_id_values(title: &Title, source: &str) -> Vec<String> {
    title
        .external_ids
        .iter()
        .filter(|external_id| external_id.source == source)
        .map(|external_id| external_id.value.clone())
        .collect()
}

/// `(season, episode, tvdb_id, tmdb_id)` for every stored episode, sorted.
async fn stored_episodes(
    app: &AppUseCase,
    user: &User,
    title_id: &str,
) -> Vec<(String, String, Option<String>, Option<String>)> {
    let mut episodes = Vec::new();
    for collection in app
        .list_collections(user, title_id)
        .await
        .expect("collections should load")
    {
        for episode in app
            .list_episodes(user, &collection.id)
            .await
            .expect("episodes should load")
        {
            episodes.push((
                episode.season_number.unwrap_or_default(),
                episode.episode_number.unwrap_or_default(),
                episode.tvdb_id,
                episode.tmdb_id,
            ));
        }
    }
    episodes.sort();
    episodes
}

#[tokio::test]
async fn a_tmdb_primary_series_hydrates_by_its_smg_title_id() {
    let smg_id = 3_100_001;
    let tmdb_id = 71_001;
    let gateway = Arc::new(SeriesTitleGateway {
        titles: vec![tmdb_primary_series(smg_id, tmdb_id, "TMDB Only Show")],
        ..Default::default()
    });
    let (app, user, _) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let created = app
        .add_title_with_outcome(
            &user,
            series_title(
                "TMDB Only Show",
                vec![ExternalId::with_kind("smg", "title", smg_id.to_string())],
            ),
        )
        .await
        .expect("title should be created")
        .title;

    let hydrated = app
        .hydrate_title_single_apq_with_language(target(&created), "eng")
        .await
        .expect("a TMDB-primary series should hydrate by SMG title id");

    assert!(hydrated.metadata_fetched_at.is_some());
    assert_eq!(hydrated.name, "TMDB Only Show");
    assert_eq!(
        external_id_values(&hydrated, "smg"),
        vec![smg_id.to_string()]
    );
    assert_eq!(
        external_id_values(&hydrated, "tmdb"),
        vec![tmdb_id.to_string()]
    );
    assert!(external_id_values(&hydrated, "tvdb").is_empty());
    assert_eq!(
        stored_episodes(&app, &user, &hydrated.id).await,
        vec![
            (
                "1".to_string(),
                "1".to_string(),
                None,
                Some((tmdb_id * 100 + 1).to_string())
            ),
            (
                "1".to_string(),
                "2".to_string(),
                None,
                Some((tmdb_id * 100 + 2).to_string())
            ),
        ]
    );
    let title_calls = gateway.title_calls.lock().await;
    assert_eq!(title_calls.len(), 1);
    assert_eq!(title_calls[0].0[0].smg_id, Some(smg_id));
    assert!(
        title_calls[0].1 && title_calls[0].2,
        "single hydration asks for episodes and orders"
    );
    assert!(gateway.metadata_bulk_calls.lock().await.is_empty());
}

#[tokio::test]
async fn a_tvdb_series_hydrates_identically_by_tvdb_ref_or_smg_id() {
    let tvdb_id = 81_001;
    let smg_id = 3_200_001;

    let mut outcomes = Vec::new();
    for external_ids in [
        // Not yet linked to SMG: resolved by its TVDB ref on the title surface.
        vec![ExternalId::new("tvdb".to_string(), tvdb_id.to_string())],
        // Already linked: addressed by SMG title id.
        vec![
            ExternalId::new("tvdb".to_string(), tvdb_id.to_string()),
            ExternalId::with_kind("smg", "title", smg_id.to_string()),
        ],
    ] {
        let gateway = Arc::new(SeriesTitleGateway {
            titles: vec![tvdb_series(tvdb_id, Some(smg_id), "TVDB Show")],
            ..Default::default()
        });
        let (app, user, _) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
        let created = app
            .add_title_with_outcome(&user, series_title("TVDB Show", external_ids))
            .await
            .expect("title should be created")
            .title;
        let hydrated = app
            .hydrate_title_single_apq_with_language(target(&created), "eng")
            .await
            .expect("a TVDB series should hydrate through the title surface");
        let title_calls = gateway.title_calls.lock().await;
        assert_eq!(title_calls.len(), 1);
        assert_eq!(title_calls[0].0[0].tvdb_id, Some(tvdb_id));
        assert!(gateway.metadata_bulk_calls.lock().await.is_empty());
        outcomes.push((
            hydrated.name.clone(),
            external_id_values(&hydrated, "tvdb"),
            external_id_values(&hydrated, "tmdb"),
            external_id_values(&hydrated, "smg"),
            stored_episodes(&app, &user, &hydrated.id).await,
        ));
    }

    assert_eq!(outcomes[0], outcomes[1]);
    let outcome = &outcomes[0];
    assert_eq!(outcome.0, "TVDB Show");
    assert_eq!(outcome.1, vec![tvdb_id.to_string()]);
    assert_eq!(outcome.3, vec![smg_id.to_string()]);
    assert_eq!(
        outcome.4,
        vec![
            (
                "1".to_string(),
                "1".to_string(),
                Some((tvdb_id * 100 + 1).to_string()),
                None
            ),
            (
                "1".to_string(),
                "2".to_string(),
                Some((tvdb_id * 100 + 2).to_string()),
                None
            ),
        ]
    );
}

#[tokio::test]
async fn a_tvdb_series_the_title_surface_cannot_resolve_fails_clearly() {
    let tvdb_id = 81_002;
    let gateway = Arc::new(SeriesTitleGateway::default());
    let (app, user, _) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let created = app
        .add_title_with_outcome(
            &user,
            series_title(
                "Unseeded Show",
                vec![ExternalId::new("tvdb".to_string(), tvdb_id.to_string())],
            ),
        )
        .await
        .expect("title should be created")
        .title;

    let error = app
        .hydrate_title_single_apq_with_language(target(&created), "eng")
        .await
        .expect_err("a series SMG cannot resolve has no fallback");
    assert!(
        error
            .to_string()
            .contains("SMG could not resolve the series from its external ids"),
        "unexpected error: {error}"
    );
    assert!(gateway.metadata_bulk_calls.lock().await.is_empty());
}

#[tokio::test]
async fn a_redirected_series_smg_id_is_persisted() {
    let old_smg_id = 3_300_001;
    let new_smg_id = 3_300_002;
    let tmdb_id = 71_003;
    let gateway = Arc::new(SeriesTitleGateway {
        titles: vec![tmdb_primary_series(new_smg_id, tmdb_id, "Merged Show")],
        redirects: vec![(old_smg_id, new_smg_id)],
        ..Default::default()
    });
    let (app, user, _) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let created = app
        .add_title_with_outcome(
            &user,
            series_title(
                "Merged Show",
                vec![ExternalId::with_kind(
                    "smg",
                    "title",
                    old_smg_id.to_string(),
                )],
            ),
        )
        .await
        .expect("title should be created")
        .title;

    let outcome = app
        .hydrate_titles_bulk(vec![target(&created)])
        .await
        .expect("the batch should run");
    let hydrated = outcome
        .hydrated_titles
        .get(&created.id)
        .expect("the redirected series should hydrate");
    assert_eq!(
        external_id_values(hydrated, "smg"),
        vec![new_smg_id.to_string()]
    );

    let stored = app
        .get_title(&user, &created.id)
        .await
        .expect("title should load")
        .expect("title should exist");
    assert_eq!(
        external_id_values(&stored, "smg"),
        vec![new_smg_id.to_string()]
    );
}

#[tokio::test]
async fn a_series_missing_from_smg_without_a_tvdb_id_fails_clearly() {
    let smg_id = 3_400_001;
    let gateway = Arc::new(SeriesTitleGateway::default());
    let (app, user, _) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let created = app
        .add_title_with_outcome(
            &user,
            series_title(
                "Vanished Show",
                vec![ExternalId::with_kind("smg", "title", smg_id.to_string())],
            ),
        )
        .await
        .expect("title should be created")
        .title;

    let error = app
        .hydrate_title_single_apq_with_language(target(&created), "eng")
        .await
        .expect_err("a series SMG no longer has cannot hydrate");
    assert!(
        error
            .to_string()
            .contains(&format!("SMG has no series for title id {smg_id}")),
        "unexpected error: {error}"
    );

    let outcome = app
        .hydrate_titles_bulk(vec![target(&created)])
        .await
        .expect("the batch should run");
    assert_eq!(
        outcome
            .failed_titles
            .get(&created.id)
            .map(|failure| failure.message.as_str()),
        Some("bulk metadata response missing title")
    );
    assert!(gateway.metadata_bulk_calls.lock().await.is_empty());
}

#[tokio::test]
async fn bulk_series_hydration_serves_both_surfaces_and_skips_episode_orders() {
    let tmdb_smg_id = 3_500_001;
    let tmdb_id = 71_005;
    let tvdb_id = 81_005;
    let tvdb_smg_id = 3_500_002;
    let gateway = Arc::new(SeriesTitleGateway {
        titles: vec![
            tmdb_primary_series(tmdb_smg_id, tmdb_id, "Bulk TMDB Show"),
            tvdb_series(tvdb_id, Some(tvdb_smg_id), "Bulk TVDB Show"),
        ],
        ..Default::default()
    });
    let (app, user, _) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let tmdb_title = app
        .add_title_with_outcome(
            &user,
            series_title(
                "Bulk TMDB Show",
                vec![ExternalId::with_kind(
                    "smg",
                    "title",
                    tmdb_smg_id.to_string(),
                )],
            ),
        )
        .await
        .expect("title should be created")
        .title;
    let tvdb_title = app
        .add_title_with_outcome(
            &user,
            series_title(
                "Bulk TVDB Show",
                vec![ExternalId::new("tvdb".to_string(), tvdb_id.to_string())],
            ),
        )
        .await
        .expect("title should be created")
        .title;

    let outcome = app
        .hydrate_titles_bulk(vec![target(&tmdb_title), target(&tvdb_title)])
        .await
        .expect("the batch should run");
    assert!(
        outcome.failed_titles.is_empty(),
        "{:?}",
        outcome.failed_titles
    );
    let hydrated_tmdb = outcome
        .hydrated_titles
        .get(&tmdb_title.id)
        .expect("TMDB series hydrates");
    let hydrated_tvdb = outcome
        .hydrated_titles
        .get(&tvdb_title.id)
        .expect("TVDB series hydrates");
    assert_eq!(
        external_id_values(hydrated_tmdb, "tmdb"),
        vec![tmdb_id.to_string()]
    );
    assert_eq!(
        external_id_values(hydrated_tvdb, "tvdb"),
        vec![tvdb_id.to_string()]
    );
    assert_eq!(stored_episodes(&app, &user, &tmdb_title.id).await.len(), 2);
    assert_eq!(stored_episodes(&app, &user, &tvdb_title.id).await.len(), 2);

    let title_calls = gateway.title_calls.lock().await;
    assert_eq!(title_calls.len(), 1, "one titles request for the chunk");
    assert!(title_calls[0].1, "bulk hydration asks for episodes");
    assert!(
        !title_calls[0].2,
        "bulk hydration never asks for episode orders"
    );
    assert!(gateway.metadata_bulk_calls.lock().await.is_empty());
}

#[tokio::test]
async fn series_hydration_fails_clearly_when_the_title_surface_errors() {
    let tvdb_id = 81_006;
    let smg_id = 3_600_001;
    let gateway = Arc::new(SeriesTitleGateway {
        failing: true,
        ..Default::default()
    });
    let (app, user, _) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let tvdb_title = app
        .add_title_with_outcome(
            &user,
            series_title(
                "TVDB Bulk Show",
                vec![ExternalId::new("tvdb".to_string(), tvdb_id.to_string())],
            ),
        )
        .await
        .expect("title should be created")
        .title;
    let tmdb_title = app
        .add_title_with_outcome(
            &user,
            series_title(
                "TMDB Bulk Show",
                vec![ExternalId::with_kind("smg", "title", smg_id.to_string())],
            ),
        )
        .await
        .expect("title should be created")
        .title;

    let error = app
        .hydrate_title_single_apq_with_language(target(&tvdb_title), "eng")
        .await
        .expect_err("a TVDB series surfaces the title-surface error");
    assert!(
        error.to_string().contains("fixture title surface failure"),
        "unexpected error: {error}"
    );

    let outcome = app
        .hydrate_titles_bulk(vec![target(&tvdb_title), target(&tmdb_title)])
        .await
        .expect("a gateway error fails the titles, not the batch");
    assert!(outcome.hydrated_titles.is_empty());
    for title in [&tvdb_title, &tmdb_title] {
        let reason = outcome
            .failed_titles
            .get(&title.id)
            .expect("every series in the chunk fails");
        assert!(reason.contains("fixture title surface failure"), "{reason}");
    }
    assert!(gateway.metadata_bulk_calls.lock().await.is_empty());
}
