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
    legacy_search_limits: Mutex<Vec<i32>>,
    combined_search_limits: Mutex<Vec<i32>>,
    combined_search_empty: bool,
    combined_search_error: Option<String>,
    /// A non-capability gateway failure for `searchTitles`, if the test wants one.
    title_search_error: Option<String>,
}

impl RecordingSearchMetadataGateway {
    async fn title_search_limits(&self) -> Vec<i32> {
        self.title_search_limits.lock().await.clone()
    }

    async fn legacy_search_limits(&self) -> Vec<i32> {
        self.legacy_search_limits.lock().await.clone()
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
        self.legacy_search_limits.lock().await.push(limit);
        Ok(vec![search_item("Legacy Movie")])
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        self.legacy_search_limits.lock().await.push(limit);
        Ok(MultiMetadataSearchResult {
            movies: vec![search_item("Legacy Movie")],
            series: vec![search_item("Legacy Series")],
            anime: vec![search_item("Legacy Anime")],
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

    async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        Err(AppError::NotFound("series".into()))
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
        assert!(gateway.legacy_search_limits().await.is_empty());
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
    assert!(gateway.legacy_search_limits().await.is_empty());
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
    assert!(gateway.legacy_search_limits().await.is_empty());
}

/// A validation error is not a capability error, so before the limit was clamped
/// this path failed the entire search. Prove the whole range the public contract
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
