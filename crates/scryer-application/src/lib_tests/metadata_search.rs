use super::*;

fn search_item(name: &str) -> RichMetadataSearchItem {
    RichMetadataSearchItem {
        tvdb_id: "4242".to_string(),
        smg_id: None,
        primary_source: None,
        external_ids: vec![],
        name: name.to_string(),
        imdb_id: None,
        slug: None,
        type_hint: None,
        year: Some(2020),
        status: None,
        overview: None,
        popularity: None,
        poster_url: None,
        language: None,
        runtime_minutes: None,
        sort_title: None,
    }
}

/// Records the limit each search call received and lets a test decide how
/// `searchTitles` answers.
#[derive(Default)]
struct RecordingSearchMetadataGateway {
    title_search_limits: Mutex<Vec<i32>>,
    tvdb_search_limits: Mutex<Vec<i32>>,
    combined_search_limits: Mutex<Vec<i32>>,
    combined_search_empty: bool,
    combined_search_error: Option<String>,
    /// A gateway failure for `searchTitles`, if the test wants one.
    title_search_error: Option<String>,
}

impl RecordingSearchMetadataGateway {
    async fn title_search_limits(&self) -> Vec<i32> {
        self.title_search_limits.lock().await.clone()
    }

    async fn tvdb_search_limits(&self) -> Vec<i32> {
        self.tvdb_search_limits.lock().await.clone()
    }
}

#[async_trait]
impl MetadataGateway for RecordingSearchMetadataGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Ok(Vec::new())
    }

    async fn search_tvdb_batch(
        &self,
        _queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        Ok(HashMap::new())
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        self.tvdb_search_limits.lock().await.push(limit);
        Ok(vec![search_item("TVDB Movie")])
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        self.tvdb_search_limits.lock().await.push(limit);
        Ok(MultiMetadataSearchResult {
            movies: vec![search_item("TVDB Movie")],
            series: vec![search_item("TVDB Series")],
            anime: vec![search_item("TVDB Anime")],
        })
    }

    async fn search_titles_multi(
        &self,
        _query: &str,
        limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        self.combined_search_limits.lock().await.push(limit);
        if let Some(message) = &self.combined_search_error {
            return Err(AppError::Repository(message.clone()));
        }
        let mut movie = search_item("TMDB Movie");
        movie.tvdb_id.clear();
        movie.smg_id = Some(123);
        movie.primary_source = Some("tmdb".into());
        Ok(if self.combined_search_empty {
            MultiMetadataSearchResult {
                movies: vec![],
                series: vec![],
                anime: vec![],
            }
        } else {
            MultiMetadataSearchResult {
                movies: vec![movie],
                series: vec![search_item("Combined Series")],
                anime: vec![search_item("Combined Anime")],
            }
        })
    }

    async fn search_titles(
        &self,
        _query: &str,
        _kind: &str,
        limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        self.title_search_limits.lock().await.push(limit);
        match self.title_search_error.as_deref() {
            Some(message) => Err(AppError::Repository(message.to_string())),
            None => Ok(vec![search_item("Title Surface Movie")]),
        }
    }

    async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        Err(AppError::NotFound("movie".into()))
    }

    async fn get_metadata_bulk(
        &self,
        _movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        Ok(BulkMetadataResult::default())
    }
}

/// Scryer's public search contract documents and clamps `limit` to 1..=100 and
/// passes it through. The gateway caps `searchTitles` lower, but that cap is the
/// gateway's business: a limit the public contract accepts must return results,
/// not a validation error.
#[tokio::test]
async fn movie_search_with_the_maximum_public_limit_succeeds() {
    let gateway = Arc::new(RecordingSearchMetadataGateway::default());
    let (app, user, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());

    let results = app
        .search_metadata(&user, "fixture", "movie", 100, "eng", None)
        .await
        .expect("a limit inside the public range must not fail the search");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Title Surface Movie");
    assert_eq!(gateway.title_search_limits().await, vec![100]);
}

#[tokio::test]
async fn multi_search_uses_only_the_combined_gateway_call() {
    for empty in [false, true] {
        let gateway = Arc::new(RecordingSearchMetadataGateway {
            combined_search_empty: empty,
            ..Default::default()
        });
        let (app, user, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
        let results = app
            .search_metadata_multi(&user, "fixture", 100, "jpn")
            .await
            .unwrap();
        assert_eq!(
            gateway.combined_search_limits.lock().await.as_slice(),
            &[100]
        );
        assert!(gateway.title_search_limits().await.is_empty());
        assert!(gateway.tvdb_search_limits().await.is_empty());
        if empty {
            assert!(
                results.movies.is_empty() && results.series.is_empty() && results.anime.is_empty()
            );
        } else {
            assert_eq!(results.movies[0].smg_id, Some(123));
            assert!(results.movies[0].tvdb_id.is_empty());
            assert_eq!(results.movies[0].primary_source.as_deref(), Some("tmdb"));
            assert_eq!(results.series[0].name, "Combined Series");
            assert_eq!(results.anime[0].name, "Combined Anime");
        }
    }
}

#[tokio::test]
async fn multi_search_does_not_add_requests_after_a_combined_search_failure() {
    let gateway = Arc::new(RecordingSearchMetadataGateway {
        combined_search_error: Some("upstream unavailable".into()),
        ..Default::default()
    });
    let (app, user, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    assert!(
        app.search_metadata_multi(&user, "fixture", 25, "eng")
            .await
            .is_err()
    );
    assert_eq!(
        gateway.combined_search_limits.lock().await.as_slice(),
        &[25]
    );
    assert!(gateway.title_search_limits().await.is_empty());
    assert!(gateway.tvdb_search_limits().await.is_empty());
}

#[tokio::test]
async fn multi_search_with_the_maximum_public_limit_succeeds() {
    let gateway = Arc::new(RecordingSearchMetadataGateway::default());
    let (app, user, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());

    let results = app
        .search_metadata_multi(&user, "fixture", 100, "eng")
        .await
        .expect("a limit inside the public range must not fail multi-search");

    assert_eq!(results.movies.len(), 1);
    assert_eq!(results.movies[0].name, "TMDB Movie");
    assert_eq!(results.series.len(), 1);
    assert_eq!(results.anime.len(), 1);
    assert_eq!(
        gateway.combined_search_limits.lock().await.as_slice(),
        &[100]
    );
    assert!(gateway.title_search_limits().await.is_empty());
    assert!(gateway.tvdb_search_limits().await.is_empty());
}

/// A limit the gateway rejects fails the entire search. Prove the whole range the public contract
/// accepts reaches the gateway.
#[tokio::test]
async fn movie_search_passes_every_publicly_accepted_limit_to_the_gateway() {
    let gateway = Arc::new(RecordingSearchMetadataGateway::default());
    let (app, user, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());

    for limit in [1, 25, 26, 100] {
        app.search_metadata(&user, "fixture", "movie", limit, "eng", None)
            .await
            .unwrap_or_else(|error| panic!("limit {limit} should succeed: {error}"));
    }

    assert_eq!(gateway.title_search_limits().await, vec![1, 25, 26, 100]);
}

fn denied_library_access(error: &AppError) -> bool {
    matches!(
        error,
        AppError::UnauthorizedReason { reason, .. }
            if *reason == crate::error_reasons::services::LIBRARY_ACCESS_DENIED
    )
}

/// A request-only actor reaches metadata search and the series lookup the
/// request dialog uses; an actor with no library grant is still refused, with
/// the same reason code as before.
#[tokio::test]
async fn request_only_actor_can_search_metadata_and_look_up_series() {
    let gateway = Arc::new(RecordingSearchMetadataGateway::default());
    let (app, _admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let movie_library = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let series_ref = SeriesTitleRef {
        tvdb_id: Some(4242),
        ..SeriesTitleRef::default()
    };

    for permission in [
        scryer_domain::LibraryPermission::Request,
        // Auto-approve implies Request.
        scryer_domain::LibraryPermission::AutoApproveRequests,
        scryer_domain::LibraryPermission::View,
        scryer_domain::LibraryPermission::ManageTitles,
    ] {
        let actor = library_permission_user("fixture-searcher", &movie_library, &[permission]);
        app.search_metadata(&actor, "fixture", "movie", 10, "eng", None)
            .await
            .unwrap_or_else(|error| panic!("{permission:?} should search: {error:?}"));
        app.search_metadata_multi(&actor, "fixture", 10, "eng")
            .await
            .unwrap_or_else(|error| panic!("{permission:?} should multi-search: {error:?}"));
        // The fixture gateway knows no series; the point is that the gate
        // lets the call through to it.
        if let Err(error) = app
            .get_metadata_series_by_ref(&actor, &series_ref, "eng", false)
            .await
        {
            assert!(
                !denied_library_access(&error),
                "{permission:?} should pass the series lookup gate: {error:?}"
            );
        }
    }

    // Grants that are none of View, Request or ManageTitles do not open it.
    for grants in [
        vec![],
        vec![scryer_domain::LibraryPermission::ManageSubtitles],
    ] {
        let actor = library_permission_user("fixture-outsider", &movie_library, &grants);
        let error = app
            .search_metadata(&actor, "fixture", "movie", 10, "eng", None)
            .await
            .expect_err("no search grant should be refused");
        assert!(denied_library_access(&error), "{error:?}");
        let error = app
            .search_metadata_multi(&actor, "fixture", 10, "eng")
            .await
            .expect_err("no search grant should be refused");
        assert!(denied_library_access(&error), "{error:?}");
        let error = app
            .get_metadata_series_by_ref(&actor, &series_ref, "eng", false)
            .await
            .expect_err("no search grant should be refused");
        assert!(denied_library_access(&error), "{error:?}");
    }
    assert_eq!(
        gateway.title_search_limits().await.len(),
        4,
        "refused calls never reach the gateway"
    );
}

fn fixture_title(id: &str, facet: MediaFacet, library_id: &str, tvdb_id: i64) -> Title {
    let mut title = make_due_hydration_title(id, facet, tvdb_id);
    title.library_id = library_id.to_string();
    title
}

/// Presence lists only libraries of the result's facet that the actor may
/// view, request into or manage titles in, in one read for every result.
#[tokio::test]
async fn library_presence_is_scoped_to_the_actors_libraries_of_the_result_facet() {
    let gateway = Arc::new(RecordingSearchMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let movie_default = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let series_default = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let anime_default = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    let movie_alt = "fixture-movie-alt";
    let movie_hidden = "fixture-movie-hidden";
    {
        let libraries = titles.library_store.get().expect("fixture libraries");
        let mut libraries = libraries.lock().await;
        libraries.push(custom_movie_library(movie_alt, "Fixture Alt"));
        libraries.push(custom_movie_library(movie_hidden, "Fixture Hidden"));
    }
    {
        let mut store = titles.store.lock().await;
        store.push(fixture_title(
            "fixture-m1",
            MediaFacet::Movie,
            &movie_default,
            7001,
        ));
        store.push(fixture_title(
            "fixture-m2",
            MediaFacet::Movie,
            movie_alt,
            7001,
        ));
        store.push(fixture_title(
            "fixture-m3",
            MediaFacet::Movie,
            movie_hidden,
            7001,
        ));
        store.push(fixture_title(
            "fixture-m4",
            MediaFacet::Movie,
            movie_hidden,
            7002,
        ));
        store.push(fixture_title(
            "fixture-s1",
            MediaFacet::Series,
            &series_default,
            7001,
        ));
        store.push(fixture_title(
            "fixture-a1",
            MediaFacet::Anime,
            &anime_default,
            7001,
        ));
    }
    let actor = library_permission_user_with_grants(
        "fixture-requester",
        &[
            (
                movie_default.as_str(),
                &[scryer_domain::LibraryPermission::Request],
            ),
            (movie_alt, &[scryer_domain::LibraryPermission::View]),
            (
                series_default.as_str(),
                &[scryer_domain::LibraryPermission::ManageTitles],
            ),
            (
                anime_default.as_str(),
                &[scryer_domain::LibraryPermission::ManageSubtitles],
            ),
        ],
    );
    let tvdb = |value: &str| vec![ExternalId::new(" TVDB ".to_string(), format!(" {value} "))];
    let held = tvdb("7001");
    let hidden_only = tvdb("7002");
    let unknown = tvdb("7999");
    let items = [
        (MediaFacet::Movie, held.as_slice()),
        (MediaFacet::Series, held.as_slice()),
        (MediaFacet::Anime, held.as_slice()),
        (MediaFacet::Movie, hidden_only.as_slice()),
        (MediaFacet::Movie, unknown.as_slice()),
        (MediaFacet::Movie, &[][..]),
    ];

    let presence = app
        .metadata_search_library_presence(&actor, &items)
        .await
        .expect("presence should resolve");

    assert_eq!(
        presence,
        vec![
            vec![movie_default.clone(), movie_alt.to_string()],
            vec![series_default.clone()],
            vec![],
            vec![],
            vec![],
            vec![],
        ]
    );
    let reads = titles
        .external_id_presence_reads
        .lock()
        .expect("presence read log")
        .clone();
    assert_eq!(reads.len(), 1, "one read covers every result");
    assert!(
        !reads[0]
            .iter()
            .any(|library_id| library_id == movie_hidden || *library_id == anime_default),
        "a library the actor holds no search grant on is never asked about: {reads:?}"
    );
}

#[tokio::test]
async fn library_presence_reads_nothing_without_an_eligible_library() {
    let gateway = Arc::new(RecordingSearchMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let movie_default = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    titles.store.lock().await.push(fixture_title(
        "fixture-m1",
        MediaFacet::Movie,
        &movie_default,
        7001,
    ));
    let held = vec![ExternalId::new("tvdb".to_string(), "7001".to_string())];
    let items = [(MediaFacet::Movie, held.as_slice())];

    let outsider = library_permission_user(
        "fixture-outsider",
        &movie_default,
        &[scryer_domain::LibraryPermission::ManageSubtitles],
    );
    assert_eq!(
        app.metadata_search_library_presence(&outsider, &items)
            .await
            .expect("presence should resolve"),
        vec![Vec::<String>::new()]
    );

    // Eligible only for series: a movie result asks nothing either.
    let series_only = library_permission_user(
        "fixture-series-requester",
        &scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
        &[scryer_domain::LibraryPermission::Request],
    );
    assert_eq!(
        app.metadata_search_library_presence(&series_only, &items)
            .await
            .expect("presence should resolve"),
        vec![Vec::<String>::new()]
    );

    assert!(
        titles
            .external_id_presence_reads
            .lock()
            .expect("presence read log")
            .is_empty()
    );
}
