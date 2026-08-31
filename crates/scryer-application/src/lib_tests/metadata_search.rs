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
    /// A non-capability gateway failure for `searchTitles`, if the test wants one.
    title_search_error: Option<String>,
}

impl RecordingSearchMetadataGateway {
    fn failing_title_search(message: &str) -> Self {
        Self {
            title_search_error: Some(message.to_string()),
            ..Default::default()
        }
    }

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
async fn multi_search_with_the_maximum_public_limit_succeeds() {
    let gateway = Arc::new(RecordingSearchMetadataGateway::default());
    let (app, user, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());

    let results = app
        .search_metadata_multi(&user, "fixture", 100, "eng")
        .await
        .expect("a limit inside the public range must not fail multi-search");

    assert_eq!(results.movies.len(), 1);
    assert_eq!(results.movies[0].name, "Title Surface Movie");
    assert_eq!(results.series.len(), 1);
    assert_eq!(results.anime.len(), 1);
    assert_eq!(gateway.title_search_limits().await, vec![100]);
    assert_eq!(gateway.legacy_search_limits().await, vec![100]);
}

/// The legacy multi-search already answered for every facet before the movie
/// title search runs. A non-capability failure of that added call must not throw
/// the series and anime results away with it.
#[tokio::test]
async fn multi_search_keeps_legacy_results_when_the_title_search_fails() {
    let gateway = Arc::new(RecordingSearchMetadataGateway::failing_title_search(
        "metadata gateway request failed (503): upstream down",
    ));
    let (app, user, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());

    let results = app
        .search_metadata_multi(&user, "fixture", 25, "eng")
        .await
        .expect("a failed movie title search must not fail the whole multi-search");

    assert_eq!(results.movies.len(), 1);
    assert_eq!(
        results.movies[0].name, "Legacy Movie",
        "the legacy movie bucket survives the failed title search"
    );
    assert_eq!(results.series.len(), 1);
    assert_eq!(results.anime.len(), 1);
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
