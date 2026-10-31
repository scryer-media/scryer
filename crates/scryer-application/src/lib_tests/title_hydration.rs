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
        ..Default::default()
    }
}

#[tokio::test]
async fn movie_defaults_remain_inherited_and_reject_unassigned_markets() {
    let (app, user, _) = bootstrap_with_metadata_gateway_and_titles(Arc::new(
        MovieTitleResolutionGateway::default(),
    ));
    let inherited = app
        .add_title_with_outcome(&user, hydration_test_title("Inherited", 991001))
        .await
        .unwrap()
        .title;
    let mut explicit_request = hydration_test_title("Explicit", 991002);
    explicit_request.min_availability = Some("in_cinemas".into());
    let explicit = app
        .add_title_with_outcome(&user, explicit_request)
        .await
        .unwrap()
        .title;
    assert_eq!(inherited.min_availability, None);
    assert_eq!(explicit.min_availability.as_deref(), Some("in_cinemas"));

    for market in ["AA", "ZZ", "UK", "USA", "1A"] {
        assert!(
            matches!(
                app.update_media_settings(
                    &user,
                    MediaFacet::Movie,
                    movie_availability_update(None, Some(market), None),
                )
                .await,
                Err(AppError::Validation(_)
                    | AppError::ValidationRefused { .. }
                    | AppError::ValidationRefusedWithCount { .. })
            ),
            "{market}"
        );
    }
    let saved = app
        .update_media_settings(
            &user,
            MediaFacet::Movie,
            movie_availability_update(Some("announced"), Some(" gb "), None),
        )
        .await
        .unwrap();
    assert_eq!(saved.movie_availability.unwrap().release_market, "GB");
    assert_eq!(
        app.get_movie_availability_for_title(&user, &inherited.id)
            .await
            .unwrap()
            .unwrap()
            .minimum_availability,
        "announced"
    );
    assert_eq!(
        app.get_movie_availability_for_title(&user, &explicit.id)
            .await
            .unwrap()
            .unwrap()
            .minimum_availability,
        "in_cinemas"
    );
}

#[tokio::test]
async fn in_flight_movie_hydration_preserves_a_changed_market_refresh() {
    for change_market in [false, true] {
        let mut movie = hydration_test_movie(991003, "Market Race");
        movie.movie_release_dates = Some(scryer_domain::MovieReleaseDates {
            market: "US".into(),
            fetched: true,
            ..Default::default()
        });
        let entered = Arc::new(tokio::sync::Notify::new());
        let resume = Arc::new(tokio::sync::Notify::new());
        let gateway = Arc::new(MovieTitleResolutionGateway {
            hydration_movies: HashMap::from([(991003, movie)]),
            hydration_gate: Some((entered.clone(), resume.clone())),
            ..Default::default()
        });
        let (app, user, titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
        let title = app
            .add_title_with_outcome(&user, hydration_test_title("Market Race", 991003))
            .await
            .unwrap()
            .title;
        let title_id = title.id.clone();
        let hydration_app = app.clone();
        let hydration = tokio::spawn(async move {
            hydration_app
                .hydrate_titles_bulk(vec![crate::catalog_workflow::HydrationTarget {
                    title,
                    requested_tvdb_id: None,
                    requested_movie_ref: None,
                    sync_wanted_after_completion: false,
                    source: crate::catalog_workflow::HydrationSource::Interactive,
                }])
                .await
        });
        within_deadline("the movie request to start", entered.notified()).await;
        if change_market {
            app.update_media_settings(
                &user,
                MediaFacet::Movie,
                movie_availability_update(None, Some("GB"), None),
            )
            .await
            .unwrap();
        }
        // The settings refresh must precede the old response's metadata write.
        app.services
            .catalog
            .titles
            .mark_title_metadata_hydration_due_now(&title_id)
            .await
            .unwrap();
        resume.notify_one();
        let result = within_deadline("the in-flight hydration to finish", hydration)
            .await
            .unwrap()
            .unwrap();
        assert!(result.failed_titles.is_empty());
        assert!(result.hydrated_titles.contains_key(&title_id));
        assert_eq!(
            titles
                .metadata_hydration_due
                .lock()
                .await
                .contains(&title_id),
            change_market
        );
        let persisted = app
            .services
            .catalog
            .titles
            .get_by_id(&title_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(persisted.movie_release_dates.unwrap().market, "US");
    }
}

/// The effective threshold is the title's own, then its library's override,
/// then the Movie facet's; the market and delay come from the library or the
/// facet. A change to the market a library uses marks its movies for
/// rehydration.
#[tokio::test]
async fn movie_availability_resolves_title_then_library_then_facet() {
    let gateway = Arc::new(MovieTitleResolutionGateway::default());
    let (app, user, titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let inherited = app
        .add_title_with_outcome(&user, hydration_test_title("Inherits Gate", 991011))
        .await
        .unwrap()
        .title;
    let mut explicit_request = hydration_test_title("Own Gate", 991012);
    explicit_request.min_availability = Some("released".into());
    let explicit = app
        .add_title_with_outcome(&user, explicit_request)
        .await
        .unwrap()
        .title;
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    assert_eq!(inherited.library_id, library_id);
    let projection = |title_id: String| {
        let app = app.clone();
        let user = user.clone();
        async move {
            let projection = app
                .get_movie_availability_for_title(&user, &title_id)
                .await
                .unwrap()
                .unwrap();
            (projection.minimum_availability, projection.release_market)
        }
    };
    let due = |title_id: String| {
        let titles = titles.clone();
        async move {
            titles
                .metadata_hydration_due
                .lock()
                .await
                .contains(&title_id)
        }
    };

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        movie_availability_update(Some("in_cinemas"), Some("DE"), Some(3)),
    )
    .await
    .unwrap();
    assert_eq!(
        projection(inherited.id.clone()).await,
        ("in_cinemas".to_string(), "DE".to_string())
    );
    assert_eq!(
        projection(explicit.id.clone()).await,
        ("released".to_string(), "DE".to_string())
    );
    assert!(due(inherited.id.clone()).await, "the facet market changed");

    titles.metadata_hydration_due.lock().await.clear();
    let mut overrides = empty_library_settings_override();
    overrides.movie_availability = MovieAvailabilityOverrides {
        default_availability: Some("announced".into()),
        release_market: Some("GB".into()),
        delay_days: None,
    };
    let library = app
        .update_library_settings(&user, &library_id, overrides)
        .await
        .unwrap();
    assert_eq!(
        library.movie_availability,
        Some(MovieAvailabilitySettings {
            default_availability: "announced".into(),
            release_market: "GB".into(),
            delay_days: 3,
        })
    );
    assert_eq!(
        projection(inherited.id.clone()).await,
        ("announced".to_string(), "GB".to_string())
    );
    assert_eq!(
        projection(explicit.id.clone()).await,
        ("released".to_string(), "GB".to_string())
    );
    assert!(due(explicit.id.clone()).await, "the library market changed");

    titles.metadata_hydration_due.lock().await.clear();
    let mut delay_only = empty_library_settings_override();
    delay_only.movie_availability.delay_days = Some(-1);
    app.update_library_settings(&user, &library_id, delay_only)
        .await
        .unwrap();
    assert_eq!(
        projection(inherited.id.clone()).await,
        ("in_cinemas".to_string(), "DE".to_string()),
        "unset library fields inherit the facet again"
    );
    assert!(
        due(inherited.id.clone()).await,
        "the market reverted to the facet's"
    );

    titles.metadata_hydration_due.lock().await.clear();
    let mut delay_only = empty_library_settings_override();
    delay_only.movie_availability.delay_days = Some(5);
    app.update_library_settings(&user, &library_id, delay_only)
        .await
        .unwrap();
    assert!(
        !due(inherited.id.clone()).await,
        "a delay change keeps the release dates already fetched"
    );

    let mut series_override = empty_library_settings_override();
    series_override.movie_availability.release_market = Some("GB".into());
    assert!(matches!(
        app.update_library_settings(
            &user,
            &scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
            series_override,
        )
        .await,
        Err(AppError::Validation(_)
            | AppError::ValidationRefused { .. }
            | AppError::ValidationRefusedWithCount { .. })
    ));
}

fn movie_availability_update(
    default_availability: Option<&str>,
    release_market: Option<&str>,
    delay_days: Option<i32>,
) -> UpdateMediaSettings {
    UpdateMediaSettings {
        movie_availability: MovieAvailabilityOverrides {
            default_availability: default_availability.map(str::to_string),
            release_market: release_market.map(str::to_string),
            delay_days,
        },
        ..empty_update_media_settings()
    }
}

fn hydration_test_title(name: &str, tvdb_id: i64) -> NewTitle {
    NewTitle {
        name: name.to_string(),
        facet: MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        external_ids: vec![ExternalId::new("tvdb".to_string(), tvdb_id.to_string())],
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
        external_ids: vec![ExternalId::new("tmdb".to_string(), tmdb_id.to_string())],
        min_availability: None,
        ..Default::default()
    }
}

async fn wait_for_title_metadata(app: &AppUseCase, user: &User, title_id: &str) -> Title {
    wait_for("the title metadata to hydrate", || async {
        app.list_titles_unpaged(user, Some(MediaFacet::Movie), None, None)
            .await
            .expect("titles should load")
            .into_iter()
            .find(|title| title.id == title_id && title.metadata_fetched_at.is_some())
    })
    .await
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
    within_deadline("the title hydration worker to stop", handle)
        .await
        .expect("title hydration worker should not panic");
}

async fn consume_title_hydration_wake(app: &AppUseCase) {
    within_deadline(
        "adding a due title to notify the hydration worker",
        app.runtime.catalog.title_hydration_wake.notified(),
    )
    .await;
}

#[derive(Default)]
struct MovieTitleResolutionGateway {
    /// Answer every title-surface request with a gateway error.
    failing: bool,
    /// While set, identity resolution fails: with a rate limit carrying this
    /// `Retry-After` when one is given, with a plain gateway error otherwise.
    resolve_failure: std::sync::Mutex<Option<Option<std::time::Duration>>>,
    unresolved: bool,
    redirected_from: Option<i64>,
    calls: Mutex<Vec<(Vec<MovieTitleRef>, bool)>>,
    movie_title_calls: Mutex<Vec<Vec<MovieTitleRef>>>,
    hydration_movies: HashMap<i64, MovieMetadata>,
    hydration_redirects: Vec<(i64, i64)>,
    hydration_gate: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
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
        if let Some((entered, resume)) = &self.hydration_gate {
            entered.notify_one();
            within_deadline("the metadata response to be released", resume.notified()).await;
        }
        if self.failing {
            return Err(AppError::Repository("fixture title surface failure".into()));
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
        if self.failing {
            return Err(AppError::Repository("fixture title surface failure".into()));
        }
        if let Some(retry_after) = *self.resolve_failure.lock().unwrap() {
            return Err(match retry_after {
                Some(retry_after) => AppError::rate_limited_temporary_unavailable(
                    "fixture gateway rate limited",
                    Some(retry_after),
                    crate::RateLimitCooldownAction::AlreadyRecorded,
                ),
                None => AppError::Repository("fixture gateway unavailable".into()),
            });
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

/// A gateway error is reported as a failed tick, links nothing, and does not
/// switch the backfill off: the next tick asks the gateway again.
#[tokio::test]
async fn movie_smg_identity_backfill_reports_a_gateway_error_as_a_failed_tick() {
    let gateway = Arc::new(MovieTitleResolutionGateway {
        failing: true,
        ..Default::default()
    });
    let (app, user, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    app.add_title_with_outcome(&user, hydration_test_title("Gateway Failure", 951_003))
        .await
        .expect("title should be created");

    let token = tokio_util::sync::CancellationToken::new();
    for _ in 0..2 {
        let tick =
            crate::catalog::title_hydration::run_movie_smg_identity_backfill_tick(&app, &token, 1)
                .await;
        assert!(
            matches!(
                &tick,
                crate::catalog::title_hydration::MovieSmgIdentityBackfillTick::Failed(error)
                    if error.to_string().contains("fixture title surface failure")
            ),
            "a gateway error must surface as a failed tick"
        );
    }
    assert_eq!(gateway.calls.lock().await.len(), 2);
    assert!(titles.store.lock().await.iter().all(|title| {
        title
            .external_ids
            .iter()
            .all(|external_id| !external_id.source.eq_ignore_ascii_case("smg"))
    }));
}

fn backfill_clock(seconds: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(1_900_000_000 + seconds, 0).expect("fixture instant")
}

/// Run the backfill phase at `seconds` on the fixed clock and return how many
/// identity batches the gateway has been sent so far.
async fn backfill_phase_at(
    app: &AppUseCase,
    gateway: &MovieTitleResolutionGateway,
    schedule: &mut crate::catalog::title_hydration::MovieSmgIdentityBackfillSchedule,
    seconds: i64,
) -> usize {
    app.runtime
        .environment
        .set_fixed_now_for_tests(Some(backfill_clock(seconds)));
    let token = tokio_util::sync::CancellationToken::new();
    assert!(
        crate::catalog::title_hydration::run_movie_smg_identity_backfill_phase(
            app, &token, schedule
        )
        .await,
        "the phase is not cancelled"
    );
    gateway.calls.lock().await.len()
}

async fn smg_linked_titles(titles: &MockTitleRepo) -> usize {
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
        .count()
}

#[tokio::test]
async fn movie_smg_identity_backfill_backs_off_while_the_gateway_fails_and_resets_on_recovery() {
    let gateway = Arc::new(MovieTitleResolutionGateway::default());
    *gateway.resolve_failure.lock().unwrap() = Some(None);
    let (app, user, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    app.add_title_with_outcome(&user, hydration_test_title("Backoff A", 951_101))
        .await
        .expect("title should be created");
    let mut schedule = Default::default();

    assert_eq!(backfill_phase_at(&app, &gateway, &mut schedule, 0).await, 1);
    // Each failure waits out the next rung: 30 s, then 1 minute, then 5.
    assert_eq!(backfill_phase_at(&app, &gateway, &mut schedule, 5).await, 1);
    assert_eq!(
        backfill_phase_at(&app, &gateway, &mut schedule, 29).await,
        1
    );
    assert_eq!(
        backfill_phase_at(&app, &gateway, &mut schedule, 30).await,
        2
    );
    assert_eq!(
        backfill_phase_at(&app, &gateway, &mut schedule, 89).await,
        2
    );
    assert_eq!(
        backfill_phase_at(&app, &gateway, &mut schedule, 90).await,
        3
    );
    assert_eq!(
        backfill_phase_at(&app, &gateway, &mut schedule, 389).await,
        3
    );

    // The gateway recovers: the batch waiting out its backoff goes through.
    *gateway.resolve_failure.lock().unwrap() = None;
    assert_eq!(
        backfill_phase_at(&app, &gateway, &mut schedule, 390).await,
        4
    );
    assert_eq!(smg_linked_titles(&titles).await, 1);

    // A success resets the ladder: the next failure waits 30 s, not 15 minutes.
    app.add_title_with_outcome(&user, hydration_test_title("Backoff B", 951_102))
        .await
        .expect("title should be created");
    *gateway.resolve_failure.lock().unwrap() = Some(None);
    assert_eq!(
        backfill_phase_at(&app, &gateway, &mut schedule, 395).await,
        5
    );
    assert_eq!(
        backfill_phase_at(&app, &gateway, &mut schedule, 424).await,
        5
    );
    *gateway.resolve_failure.lock().unwrap() = None;
    assert_eq!(
        backfill_phase_at(&app, &gateway, &mut schedule, 425).await,
        6
    );
    assert_eq!(smg_linked_titles(&titles).await, 2);
}

#[tokio::test]
async fn movie_smg_identity_backfill_waits_out_a_gateway_retry_after() {
    let gateway = Arc::new(MovieTitleResolutionGateway::default());
    *gateway.resolve_failure.lock().unwrap() = Some(Some(std::time::Duration::from_secs(12 * 60)));
    let (app, user, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    app.add_title_with_outcome(&user, hydration_test_title("Retry After", 951_103))
        .await
        .expect("title should be created");
    let mut schedule = Default::default();

    assert_eq!(backfill_phase_at(&app, &gateway, &mut schedule, 0).await, 1);
    // Twelve minutes climbs to the first rung that covers it: fifteen.
    assert_eq!(
        backfill_phase_at(&app, &gateway, &mut schedule, 12 * 60).await,
        1
    );
    assert_eq!(
        backfill_phase_at(&app, &gateway, &mut schedule, 15 * 60 - 1).await,
        1
    );
    *gateway.resolve_failure.lock().unwrap() = None;
    assert_eq!(
        backfill_phase_at(&app, &gateway, &mut schedule, 15 * 60).await,
        2
    );
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

    wait_until("the worker to request TMDB-only movie metadata", || async {
        !gateway.movie_title_calls.lock().await.is_empty()
    })
    .await;
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
    request
        .external_ids
        .push(ExternalId::new("smg".to_string(), old_smg_id.to_string()));
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

/// A title-surface error fails every movie in the batch with the gateway's
/// message, so the worker schedules its normal retry; nothing is hydrated
/// from another document and nothing is silently parked.
#[tokio::test]
async fn bulk_movie_hydration_fails_the_batch_movies_on_a_gateway_error() {
    let tvdb_id = 901_020;
    let tmdb_id = 810_020;
    let mut movie = hydration_test_movie(tvdb_id, "Fixture TVDB Movie");
    movie.smg_id = Some(1_901_020);
    let gateway = Arc::new(MovieTitleResolutionGateway {
        failing: true,
        hydration_movies: HashMap::from([(tvdb_id, movie)]),
        ..Default::default()
    });
    let (app, user, _) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let tvdb_title = app
        .add_title_with_outcome(&user, hydration_test_title("Fixture TVDB Movie", tvdb_id))
        .await
        .expect("TVDB title should be created")
        .title;
    let tmdb_title = app
        .add_title_with_outcome(
            &user,
            hydration_test_tmdb_title("Fixture TMDB Movie", tmdb_id),
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
        .expect("a gateway error is reported per title, not as a batch error");
    assert!(outcome.hydrated_titles.is_empty());
    for title_id in [&tvdb_title.id, &tmdb_title.id] {
        assert!(
            outcome
                .failed_titles
                .get(title_id)
                .is_some_and(|reason| reason.contains("fixture title surface failure")),
            "every movie in the batch must fail with the gateway's message"
        );
    }
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

    // Let the worker reach its scan-owned yield and park on the tracker, so the
    // yield counted after the add below comes from a pass that saw the title.
    let yields = app.runtime.catalog.title_hydration_scan_yields.clone();
    wait_until(
        "the hydration worker to park behind the active scan",
        || async {
            yields.load(Ordering::SeqCst) > 0
                && app
                    .runtime
                    .library
                    .library_scan_tracker
                    .subscription_count()
                    > 0
        },
    )
    .await;
    let yields_before_add = yields.load(Ordering::SeqCst);

    let outcome = app
        .add_title_with_outcome(&user, hydration_test_title("Scan Blocked Movie", tvdb_id))
        .await
        .expect("add title should succeed");
    wait_until(
        "the hydration worker to yield again after the add woke it",
        || async { yields.load(Ordering::SeqCst) > yields_before_add },
    )
    .await;
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

/// A brand-new title can be hydrated from two lanes at once — a library scan's
/// bulk pass and an interactive one. Both must leave the title hydrated: the
/// loser of the persist race used to read back a row whose `metadata_fetched_at`
/// the other writer had already overwritten and report "metadata could not be
/// persisted" for a title that was hydrated.
#[tokio::test]
async fn concurrent_hydrations_of_one_title_both_leave_it_hydrated() {
    let tvdb_id = 901_030;
    let gateway = Arc::new(MovieTitleResolutionGateway {
        hydration_movies: HashMap::from([(
            tvdb_id,
            hydration_test_movie(tvdb_id, "Concurrent Hydration"),
        )]),
        ..Default::default()
    });
    let (app, user, _) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let created = app
        .add_title_with_outcome(
            &user,
            hydration_test_title("Concurrent Placeholder", tvdb_id),
        )
        .await
        .expect("title should be created")
        .title;
    let target = || crate::catalog_workflow::HydrationTarget {
        title: created.clone(),
        requested_tvdb_id: None,
        requested_movie_ref: None,
        sync_wanted_after_completion: false,
        source: crate::catalog_workflow::HydrationSource::Interactive,
    };

    let scan_lane = app.clone();
    let interactive_lane = app.clone();
    let (scan, interactive) = tokio::join!(
        scan_lane.hydrate_titles_bulk(vec![target()]),
        interactive_lane.hydrate_titles_bulk(vec![target()]),
    );
    for outcome in [
        scan.expect("scan hydration should not fail the batch"),
        interactive.expect("interactive hydration should not fail the batch"),
    ] {
        assert!(
            !outcome.failed_titles.contains_key(&created.id),
            "neither lane may report a persistence failure: {:?}",
            outcome.failed_titles
        );
        assert!(outcome.hydrated_titles.contains_key(&created.id));
    }

    let hydrated = app
        .get_title(&user, &created.id)
        .await
        .expect("title should load")
        .expect("title should exist");
    assert!(hydrated.metadata_fetched_at.is_some());
    assert_eq!(hydrated.name, "Concurrent Hydration");
}

/// The lane that waited applies its result to the row as it stands, not to the
/// pre-hydration struct it queued, so a late second pass cannot roll the title
/// back to its unhydrated state.
#[tokio::test]
async fn a_stale_hydration_target_does_not_unhydrate_the_title() {
    let tvdb_id = 901_031;
    let gateway = Arc::new(MovieTitleResolutionGateway {
        hydration_movies: HashMap::from([(
            tvdb_id,
            hydration_test_movie(tvdb_id, "Stale Target Hydration"),
        )]),
        ..Default::default()
    });
    let (app, user, _) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let created = app
        .add_title_with_outcome(
            &user,
            hydration_test_title("Stale Target Placeholder", tvdb_id),
        )
        .await
        .expect("title should be created")
        .title;
    let target = || crate::catalog_workflow::HydrationTarget {
        // Deliberately the pre-hydration struct for both passes.
        title: created.clone(),
        requested_tvdb_id: None,
        requested_movie_ref: None,
        sync_wanted_after_completion: false,
        source: crate::catalog_workflow::HydrationSource::Interactive,
    };

    app.hydrate_titles_bulk(vec![target()])
        .await
        .expect("first hydration should succeed");
    let outcome = app
        .hydrate_titles_bulk(vec![target()])
        .await
        .expect("second hydration should succeed");
    assert!(!outcome.failed_titles.contains_key(&created.id));

    let hydrated = app
        .get_title(&user, &created.id)
        .await
        .expect("title should load")
        .expect("title should exist");
    assert!(hydrated.metadata_fetched_at.is_some());
    assert_eq!(hydrated.name, "Stale Target Hydration");
}
