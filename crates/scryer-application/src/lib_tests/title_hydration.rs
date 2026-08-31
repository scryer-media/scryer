use super::*;

fn hydration_test_movie(tvdb_id: i64, name: &str) -> MovieMetadata {
    MovieMetadata {
        target_key: None,
        smg_id: None,
        primary_source: "tvdb".to_string(),
        tvdb_id: Some(tvdb_id),
        name: name.to_string(),
        slug: name.to_ascii_lowercase().replace(' ', "-"),
        year: Some(2026),
        content_status: "Released".to_string(),
        overview: format!("{name} overview"),
        poster_url: format!("https://example.invalid/{tvdb_id}.jpg"),
        background_url: None,
        language: "eng".to_string(),
        original_language: Some("eng".to_string()),
        runtime_minutes: 90,
        sort_title: name.to_string(),
        imdb_id: format!("tt{tvdb_id:07}"),
        tmdb_id: None,
        popularity: None,
        anidb_id: None,
        canonical_tags: vec![],
        studio: "Scryer Studios".to_string(),
        tmdb_release_date: Some("2026-01-01".to_string()),
        ratings: Default::default(),
        credits: Vec::new(),
    }
}

fn hydration_test_title(name: &str, tvdb_id: i64) -> NewTitle {
    NewTitle {
        name: name.to_string(),
        facet: MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        external_ids: vec![ExternalId {
            source: "tvdb".to_string(),
            value: tvdb_id.to_string(),
        }],
        min_availability: None,
        ..Default::default()
    }
}

fn hydration_test_tmdb_title(name: &str, tmdb_id: i64) -> NewTitle {
    NewTitle {
        name: name.to_string(),
        facet: MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        external_ids: vec![ExternalId {
            source: "tmdb".to_string(),
            value: tmdb_id.to_string(),
        }],
        min_availability: None,
        ..Default::default()
    }
}

async fn wait_for_title_metadata(app: &AppUseCase, user: &User, title_id: &str) -> Title {
    timeout(Duration::from_secs(2), async {
        loop {
            let titles = app
                .list_titles_unpaged(user, Some(MediaFacet::Movie), None, None)
                .await
                .expect("titles should load");
            if let Some(title) = titles.into_iter().find(|title| title.id == title_id)
                && title.metadata_fetched_at.is_some()
            {
                return title;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("title metadata should hydrate")
}

async fn assert_title_metadata_pending(app: &AppUseCase, user: &User, title_id: &str) {
    let titles = app
        .list_titles_unpaged(user, Some(MediaFacet::Movie), None, None)
        .await
        .expect("titles should load");
    let title = titles
        .into_iter()
        .find(|title| title.id == title_id)
        .expect("title should exist");
    assert_eq!(title.metadata_fetched_at, None);
}

async fn stop_title_hydration_worker(
    token: tokio_util::sync::CancellationToken,
    handle: tokio::task::JoinHandle<()>,
) {
    token.cancel();
    timeout(Duration::from_secs(1), handle)
        .await
        .expect("title hydration worker should stop")
        .expect("title hydration worker should not panic");
}

async fn consume_title_hydration_wake(app: &AppUseCase) {
    timeout(
        Duration::from_secs(1),
        app.runtime.catalog.title_hydration_wake.notified(),
    )
    .await
    .expect("adding a due title should notify the hydration worker");
}

#[derive(Default)]
struct MovieTitleResolutionGateway {
    unsupported: bool,
    unresolved: bool,
    /// Answer as an SMG that predates the title-id surface does: with the raw
    /// GraphQL validation error, before the client maps it to a capability error.
    raw_unknown_field_error: bool,
    redirected_from: Option<i64>,
    calls: Mutex<Vec<(Vec<MovieTitleRef>, bool)>>,
    movie_title_calls: Mutex<Vec<Vec<MovieTitleRef>>>,
    hydration_movies: HashMap<i64, MovieMetadata>,
    hydration_redirects: Vec<(i64, i64)>,
}

#[async_trait]
impl MetadataGateway for MovieTitleResolutionGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Err(AppError::Repository(
            "not used by identity backfill tests".into(),
        ))
    }

    async fn search_tvdb_batch(
        &self,
        _queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        Err(AppError::Repository(
            "not used by identity backfill tests".into(),
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
            "not used by identity backfill tests".into(),
        ))
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Err(AppError::Repository(
            "not used by identity backfill tests".into(),
        ))
    }

    async fn get_movie(&self, tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        self.hydration_movies
            .values()
            .find(|movie| movie.tvdb_id == Some(tvdb_id))
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("movie {tvdb_id}")))
    }

    async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        Err(AppError::Repository(
            "not used by identity backfill tests".into(),
        ))
    }

    async fn get_metadata_bulk(
        &self,
        movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        Ok(BulkMetadataResult {
            movies: movie_tvdb_ids
                .iter()
                .filter_map(|tvdb_id| {
                    self.hydration_movies
                        .values()
                        .find(|movie| movie.tvdb_id == Some(*tvdb_id))
                        .cloned()
                        .map(|movie| (*tvdb_id, movie))
                })
                .collect(),
            series: HashMap::new(),
        })
    }

    async fn get_movie_titles(
        &self,
        refs: &[MovieTitleRef],
        _language: &str,
    ) -> AppResult<MovieTitleBulkResult> {
        self.movie_title_calls.lock().await.push(refs.to_vec());
        if self.unsupported {
            return Err(AppError::Repository(
                "metadata gateway does not support title-id queries".into(),
            ));
        }

        let mut result = MovieTitleBulkResult {
            redirects: self.hydration_redirects.clone(),
            ..Default::default()
        };
        for (ref_index, movie_ref) in refs.iter().enumerate() {
            let movie = self.hydration_movies.values().find(|movie| {
                movie_ref
                    .smg_id
                    .is_some_and(|smg_id| movie.smg_id == Some(smg_id))
                    || movie_ref
                        .tvdb_id
                        .is_some_and(|tvdb_id| movie.tvdb_id == Some(tvdb_id))
                    || movie_ref
                        .tmdb_id
                        .is_some_and(|tmdb_id| movie.tmdb_id == Some(tmdb_id))
                    || movie_ref
                        .imdb_id
                        .as_deref()
                        .is_some_and(|imdb_id| movie.imdb_id == imdb_id)
            });
            if let Some(movie) = movie {
                result.by_ref_index.insert(ref_index, movie.clone());
            } else {
                result.missing_ref_indexes.push(ref_index);
            }
        }
        Ok(result)
    }

    async fn resolve_movie_titles(
        &self,
        refs: &[MovieTitleRef],
        create_missing: bool,
    ) -> AppResult<Vec<TitleResolution>> {
        self.calls
            .lock()
            .await
            .push((refs.to_vec(), create_missing));
        if self.raw_unknown_field_error {
            return Err(AppError::Repository(
                "Cannot query field \"resolveTitles\" on type \"Query\".".into(),
            ));
        }
        if self.unsupported {
            return Err(AppError::Repository(
                "metadata gateway does not support title-id queries".into(),
            ));
        }
        if self.unresolved {
            return Ok(refs
                .iter()
                .enumerate()
                .map(|(ref_index, _)| TitleResolution {
                    ref_index,
                    resolved: false,
                    smg_id: None,
                    kind: "movie".to_string(),
                    primary_source: String::new(),
                    redirected_from: None,
                    created: false,
                    external_ids: vec![],
                    reason: "not found".to_string(),
                })
                .collect());
        }

        Ok(refs
            .iter()
            .enumerate()
            .filter_map(|(ref_index, reference)| {
                reference.tvdb_id.map(|tvdb_id| TitleResolution {
                    ref_index,
                    resolved: true,
                    smg_id: Some(tvdb_id + 1_000_000),
                    kind: "movie".to_string(),
                    primary_source: "tvdb".to_string(),
                    redirected_from: self.redirected_from,
                    created: false,
                    external_ids: vec![],
                    reason: String::new(),
                })
            })
            .collect())
    }
}

#[tokio::test]
async fn movie_smg_identity_backfill_links_ids_and_resumes_from_its_cursor() {
    let gateway = Arc::new(MovieTitleResolutionGateway::default());
    let (app, user, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    app.add_title_with_outcome(&user, hydration_test_title("Cursor A", 951_001))
        .await
        .expect("first title should be created");
    app.add_title_with_outcome(&user, hydration_test_title("Cursor B", 951_002))
        .await
        .expect("second title should be created");
    let token = tokio_util::sync::CancellationToken::new();

    let first =
        crate::catalog::title_hydration::run_movie_smg_identity_backfill_tick(&app, &token, 1)
            .await;
    let crate::catalog::title_hydration::MovieSmgIdentityBackfillTick::Completed(summary) = first
    else {
        panic!("first backfill tick should complete");
    };
    assert_eq!(summary.linked, 1);
    assert_eq!(
        titles
            .store
            .lock()
            .await
            .iter()
            .filter(|title| {
                title
                    .external_ids
                    .iter()
                    .any(|external_id| external_id.source == "smg")
            })
            .count(),
        1
    );

    let second =
        crate::catalog::title_hydration::run_movie_smg_identity_backfill_tick(&app, &token, 1)
            .await;
    let crate::catalog::title_hydration::MovieSmgIdentityBackfillTick::Completed(summary) = second
    else {
        panic!("second backfill tick should complete");
    };
    assert_eq!(summary.linked, 1);
    assert!(titles.store.lock().await.iter().all(|title| {
        title
            .external_ids
            .iter()
            .any(|external_id| external_id.source == "smg")
    }));

    let calls = gateway.calls.lock().await;
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|(_, create_missing)| !create_missing));
    assert!(calls.iter().all(|(refs, _)| refs.len() == 1));
}

#[tokio::test]
async fn movie_smg_identity_backfill_keeps_its_cursor_after_an_unresolved_pass() {
    let gateway = Arc::new(MovieTitleResolutionGateway {
        unresolved: true,
        ..Default::default()
    });
    let (app, user, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let added = app
        .add_title_with_outcome(&user, hydration_test_title("Unresolved", 951_005))
        .await
        .expect("title should be created");
    let token = tokio_util::sync::CancellationToken::new();

    let first =
        crate::catalog::title_hydration::run_movie_smg_identity_backfill_tick(&app, &token, 1)
            .await;
    assert!(matches!(
        first,
        crate::catalog::title_hydration::MovieSmgIdentityBackfillTick::Completed(ref summary)
            if summary.unresolved == 1
    ));
    let second =
        crate::catalog::title_hydration::run_movie_smg_identity_backfill_tick(&app, &token, 1)
            .await;
    assert!(matches!(
        second,
        crate::catalog::title_hydration::MovieSmgIdentityBackfillTick::Completed(ref summary)
            if summary == &Default::default()
    ));
    assert_eq!(gateway.calls.lock().await.len(), 1);
    assert_eq!(
        titles
            .smg_identity_backfill_attempts
            .lock()
            .await
            .get(&added.title.id),
        Some(&1)
    );
}

#[tokio::test]
async fn movie_smg_identity_backfill_excludes_a_title_after_the_attempt_cap() {
    let gateway = Arc::new(MovieTitleResolutionGateway {
        unresolved: true,
        ..Default::default()
    });
    let (app, user, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let added = app
        .add_title_with_outcome(&user, hydration_test_title("Terminal", 951_006))
        .await
        .expect("title should be created");
    titles
        .smg_identity_backfill_attempts
        .lock()
        .await
        .insert(added.title.id.clone(), 4);
    let token = tokio_util::sync::CancellationToken::new();

    let first =
        crate::catalog::title_hydration::run_movie_smg_identity_backfill_tick(&app, &token, 1)
            .await;
    assert!(matches!(
        first,
        crate::catalog::title_hydration::MovieSmgIdentityBackfillTick::Completed(ref summary)
            if summary.unresolved == 1
    ));
    let second =
        crate::catalog::title_hydration::run_movie_smg_identity_backfill_tick(&app, &token, 1)
            .await;
    assert!(matches!(
        second,
        crate::catalog::title_hydration::MovieSmgIdentityBackfillTick::Completed(ref summary)
            if summary == &Default::default()
    ));
    assert_eq!(gateway.calls.lock().await.len(), 1);
    assert_eq!(
        titles
            .smg_identity_backfill_attempts
            .lock()
            .await
            .get(&added.title.id),
        Some(&5)
    );
}

#[tokio::test]
async fn movie_smg_identity_backfill_skips_the_default_not_supported_gateway_error() {
    let gateway = Arc::new(MovieTitleResolutionGateway {
        unsupported: true,
        ..Default::default()
    });
    let (app, user, titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
    app.add_title_with_outcome(&user, hydration_test_title("Unsupported", 951_003))
        .await
        .expect("title should be created");

    let token = tokio_util::sync::CancellationToken::new();
    let tick =
        crate::catalog::title_hydration::run_movie_smg_identity_backfill_tick(&app, &token, 1)
            .await;
    assert!(matches!(
        tick,
        crate::catalog::title_hydration::MovieSmgIdentityBackfillTick::NotSupported
    ));
    assert!(titles.store.lock().await.iter().all(|title| {
        title
            .external_ids
            .iter()
            .all(|external_id| !external_id.source.eq_ignore_ascii_case("smg"))
    }));
}

/// An old SMG rejects `resolveTitles` with a raw validation error naming that
/// field. Read as anything but a capability signal, the backfill worker reports
/// `Failed` on every tick forever instead of switching itself off once.
#[tokio::test]
async fn movie_smg_identity_backfill_stops_on_a_raw_unknown_field_error() {
    let gateway = Arc::new(MovieTitleResolutionGateway {
        raw_unknown_field_error: true,
        ..Default::default()
    });
    let (app, user, titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
    app.add_title_with_outcome(&user, hydration_test_title("Raw Unsupported", 951_004))
        .await
        .expect("title should be created");

    let token = tokio_util::sync::CancellationToken::new();
    let tick =
        crate::catalog::title_hydration::run_movie_smg_identity_backfill_tick(&app, &token, 1)
            .await;
    assert!(
        matches!(
            tick,
            crate::catalog::title_hydration::MovieSmgIdentityBackfillTick::NotSupported
        ),
        "a raw unknown-field error must disable the backfill, not fail the tick"
    );
    assert!(titles.store.lock().await.iter().all(|title| {
        title
            .external_ids
            .iter()
            .all(|external_id| !external_id.source.eq_ignore_ascii_case("smg"))
    }));
}

#[tokio::test]
async fn prompt_title_hydration_worker_processes_pending_title_after_wake() {
    let (app, user) = bootstrap();
    let tvdb_id = 901_001;
    let mut movie = hydration_test_movie(tvdb_id, "Wake Movie");
    movie.smg_id = Some(1_901_001);
    let app = app.with_test_overrides(|services| {
        services.with_metadata_gateway(Arc::new(MockMetadataGateway {
            movies: HashMap::from([(tvdb_id, movie)]),
        }))
    });
    let token = tokio_util::sync::CancellationToken::new();
    let handle = tokio::spawn(start_background_title_hydration_loop(
        app.clone(),
        token.clone(),
    ));

    let outcome = app
        .add_title_with_outcome(&user, hydration_test_title("Wake Movie", tvdb_id))
        .await
        .expect("add title should succeed");
    assert_eq!(
        outcome.metadata_hydration_state,
        AddTitleHydrationState::Pending
    );

    let hydrated = wait_for_title_metadata(&app, &user, &outcome.title.id).await;
    assert_eq!(hydrated.name, "Wake Movie");
    assert_eq!(hydrated.year, Some(2026));
    assert_eq!(hydrated.language.as_deref(), Some("eng"));
    assert_eq!(hydrated.metadata_language.as_deref(), Some("eng"));
    assert!(
        hydrated
            .external_ids
            .iter()
            .any(|external_id| { external_id.source == "smg" && external_id.value == "1901001" })
    );

    stop_title_hydration_worker(token, handle).await;
}

#[tokio::test]
async fn title_hydration_worker_processes_pending_title_immediately_after_startup() {
    let (app, user) = bootstrap();
    let tvdb_id = 901_003;
    let app = app.with_test_overrides(|services| {
        services.with_metadata_gateway(Arc::new(MockMetadataGateway {
            movies: HashMap::from([(
                tvdb_id,
                hydration_test_movie(tvdb_id, "Startup Pending Movie"),
            )]),
        }))
    });

    let outcome = app
        .add_title_with_outcome(
            &user,
            hydration_test_title("Startup Pending Movie", tvdb_id),
        )
        .await
        .expect("add title should succeed");
    assert_eq!(
        outcome.metadata_hydration_state,
        AddTitleHydrationState::Pending
    );
    consume_title_hydration_wake(&app).await;

    let token = tokio_util::sync::CancellationToken::new();
    let handle = tokio::spawn(start_background_title_hydration_loop(
        app.clone(),
        token.clone(),
    ));

    let hydrated = wait_for_title_metadata(&app, &user, &outcome.title.id).await;
    assert_eq!(hydrated.name, "Startup Pending Movie");

    stop_title_hydration_worker(token, handle).await;
}

#[tokio::test]
async fn title_hydration_worker_drains_multiple_pending_batches_without_pacing() {
    let (app, user) = bootstrap();
    let movies = (0..21)
        .map(|index| {
            let tvdb_id = 901_100 + index;
            (
                tvdb_id,
                hydration_test_movie(tvdb_id, &format!("Batch Pending Movie {index}")),
            )
        })
        .collect::<HashMap<_, _>>();
    let app = app.with_test_overrides(|services| {
        services.with_metadata_gateway(Arc::new(MockMetadataGateway { movies }))
    });

    let mut title_ids = Vec::new();
    for index in 0..21 {
        let tvdb_id = 901_100 + index;
        let name = format!("Batch Pending Movie {index}");
        let outcome = app
            .add_title_with_outcome(&user, hydration_test_title(&name, tvdb_id))
            .await
            .expect("add title should succeed");
        assert_eq!(
            outcome.metadata_hydration_state,
            AddTitleHydrationState::Pending
        );
        title_ids.push(outcome.title.id);
    }
    consume_title_hydration_wake(&app).await;

    let token = tokio_util::sync::CancellationToken::new();
    let handle = tokio::spawn(start_background_title_hydration_loop(
        app.clone(),
        token.clone(),
    ));

    for title_id in title_ids {
        wait_for_title_metadata(&app, &user, &title_id).await;
    }

    stop_title_hydration_worker(token, handle).await;
}

#[tokio::test]
async fn prompt_title_hydration_worker_hydrates_tmdb_only_movie_by_title_ref() {
    let tmdb_id = 810_003;
    let mut movie = hydration_test_movie(0, "TMDB Wake Movie");
    movie.smg_id = Some(1_810_003);
    movie.tvdb_id = None;
    movie.tmdb_id = Some(tmdb_id);
    movie.imdb_id = "tt8100003".to_string();
    let gateway = Arc::new(MovieTitleResolutionGateway {
        hydration_movies: HashMap::from([(tmdb_id, movie)]),
        ..Default::default()
    });
    let (app, user) = bootstrap();
    let app = app.with_test_overrides(|services| services.with_metadata_gateway(gateway.clone()));
    let token = tokio_util::sync::CancellationToken::new();
    let handle = tokio::spawn(start_background_title_hydration_loop(
        app.clone(),
        token.clone(),
    ));

    let outcome = app
        .add_title_with_outcome(&user, hydration_test_tmdb_title("TMDB Wake Movie", tmdb_id))
        .await
        .expect("TMDB-only movie should queue hydration");
    assert_eq!(
        outcome.metadata_hydration_state,
        AddTitleHydrationState::Pending
    );

    timeout(Duration::from_secs(2), async {
        loop {
            if !gateway.movie_title_calls.lock().await.is_empty() {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("worker should request TMDB-only movie metadata");
    let hydrated = wait_for_title_metadata(&app, &user, &outcome.title.id).await;
    assert_eq!(
        hydrated.poster_url.as_deref(),
        Some("https://example.invalid/0.jpg")
    );
    assert!(
        hydrated
            .external_ids
            .iter()
            .any(|external_id| { external_id.source == "smg" && external_id.value == "1810003" })
    );
    assert!(
        hydrated
            .external_ids
            .iter()
            .any(|external_id| { external_id.source == "tmdb" && external_id.value == "810003" })
    );
    assert!(
        hydrated.external_ids.iter().any(|external_id| {
            external_id.source == "imdb" && external_id.value == "tt8100003"
        })
    );
    assert_eq!(gateway.movie_title_calls.lock().await.len(), 1);

    stop_title_hydration_worker(token, handle).await;
}

#[tokio::test]
async fn bulk_movie_hydration_replaces_redirected_smg_id() {
    let tvdb_id = 901_010;
    let old_smg_id = 1_901_010;
    let new_smg_id = 1_901_011;
    let mut movie = hydration_test_movie(tvdb_id, "Redirected Movie");
    movie.smg_id = Some(new_smg_id);
    let gateway = Arc::new(MovieTitleResolutionGateway {
        hydration_movies: HashMap::from([(tvdb_id, movie)]),
        hydration_redirects: vec![(old_smg_id, new_smg_id)],
        ..Default::default()
    });
    let (app, user, _) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let mut request = hydration_test_title("Redirected Movie", tvdb_id);
    request.external_ids.push(ExternalId {
        source: "smg".to_string(),
        value: old_smg_id.to_string(),
    });
    let created = app
        .add_title_with_outcome(&user, request)
        .await
        .expect("title should be created");

    let outcome = app
        .hydrate_titles_bulk(vec![crate::catalog_workflow::HydrationTarget {
            title: created.title.clone(),
            requested_tvdb_id: None,
            requested_movie_ref: None,
            sync_wanted_after_completion: false,
            source: crate::catalog_workflow::HydrationSource::Interactive,
        }])
        .await
        .expect("movie should hydrate");
    let hydrated = outcome
        .hydrated_titles
        .get(&created.title.id)
        .expect("title should be hydrated");
    let smg_ids = hydrated
        .external_ids
        .iter()
        .filter(|external_id| external_id.source == "smg")
        .collect::<Vec<_>>();
    assert_eq!(smg_ids.len(), 1);
    assert_eq!(smg_ids[0].value, new_smg_id.to_string());
}

#[tokio::test]
async fn legacy_movie_hydration_falls_back_for_tvdb_and_defers_tmdb_only() {
    let tvdb_id = 901_020;
    let tmdb_id = 810_020;
    let mut movie = hydration_test_movie(tvdb_id, "Legacy TVDB Movie");
    movie.smg_id = Some(1_901_020);
    let gateway = Arc::new(MovieTitleResolutionGateway {
        unsupported: true,
        hydration_movies: HashMap::from([(tvdb_id, movie)]),
        ..Default::default()
    });
    let (app, user, _) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let tvdb_title = app
        .add_title_with_outcome(&user, hydration_test_title("Legacy TVDB Movie", tvdb_id))
        .await
        .expect("TVDB title should be created")
        .title;
    let tmdb_title = app
        .add_title_with_outcome(
            &user,
            hydration_test_tmdb_title("Legacy TMDB Movie", tmdb_id),
        )
        .await
        .expect("TMDB title should be created")
        .title;

    let outcome = app
        .hydrate_titles_bulk(vec![
            crate::catalog_workflow::HydrationTarget {
                title: tvdb_title.clone(),
                requested_tvdb_id: None,
                requested_movie_ref: None,
                sync_wanted_after_completion: false,
                source: crate::catalog_workflow::HydrationSource::Interactive,
            },
            crate::catalog_workflow::HydrationTarget {
                title: tmdb_title.clone(),
                requested_tvdb_id: None,
                requested_movie_ref: None,
                sync_wanted_after_completion: false,
                source: crate::catalog_workflow::HydrationSource::Interactive,
            },
        ])
        .await
        .expect("legacy fallback should not fail the batch");
    assert!(outcome.hydrated_titles.contains_key(&tvdb_title.id));
    assert!(outcome.deferred_titles.contains(&tmdb_title.id));
    assert!(!outcome.failed_titles.contains_key(&tmdb_title.id));
}

#[tokio::test]
async fn prompt_title_hydration_worker_yields_to_active_scan_facet() {
    let (app, user) = bootstrap();
    let tvdb_id = 901_002;
    let app = app.with_test_overrides(|services| {
        services.with_metadata_gateway(Arc::new(MockMetadataGateway {
            movies: HashMap::from([(tvdb_id, hydration_test_movie(tvdb_id, "Scan Blocked Movie"))]),
        }))
    });
    let scan = app
        .runtime
        .library
        .library_scan_tracker
        .start_session(MediaFacet::Movie)
        .await
        .expect("scan should start");
    let token = tokio_util::sync::CancellationToken::new();
    let handle = tokio::spawn(start_background_title_hydration_loop(
        app.clone(),
        token.clone(),
    ));

    let outcome = app
        .add_title_with_outcome(&user, hydration_test_title("Scan Blocked Movie", tvdb_id))
        .await
        .expect("add title should succeed");
    sleep(Duration::from_millis(75)).await;
    assert_title_metadata_pending(&app, &user, &outcome.title.id).await;

    app.runtime
        .library
        .library_scan_tracker
        .fail_session(&scan.session_id)
        .await
        .expect("scan should finish");
    let hydrated = wait_for_title_metadata(&app, &user, &outcome.title.id).await;
    assert_eq!(hydrated.name, "Scan Blocked Movie");

    stop_title_hydration_worker(token, handle).await;
}
