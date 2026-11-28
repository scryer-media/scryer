use super::*;

#[tokio::test]
async fn graphql_search_metadata_movie_accepts_year_hint() {
    let ctx = TestContext::new().await;
    let fixture = load_fixture("smg/search_titles_tmdb_primary.json");
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .and(query_param(
            "variables",
            r#"{"query":"Test Movie","kind":"movie","limit":25,"language":"eng","year":2024}"#,
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let body = gql(
        &ctx,
        r#"query($query: String!, $type: MediaFacetValue!, $year: Int) {
            searchMetadata(query: $query, type: $type, year: $year) {
                smgId tmdbId primarySource externalIds { source value }
                tvdbId name year type overview posterUrl
            }
        }"#,
        json!({ "query": "Test Movie", "type": "MOVIE", "year": 2024 }),
    )
    .await;
    assert_no_errors(&body);
    let results = body["data"]["searchMetadata"].as_array().unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0]["name"], "TMDB Primary Movie");
    assert_eq!(results[0]["tvdbId"], "");
    assert_eq!(results[0]["smgId"], 202);
    assert_eq!(results[0]["tmdbId"], 2020);
    assert_eq!(results[0]["primarySource"], "tmdb");
    let poster_url = results[0]["posterUrl"]
        .as_str()
        .expect("metadata search should proxy present poster artwork");
    assert!(poster_url.starts_with("/images/media/"));
    assert!(poster_url.ends_with("/w250"));
    assert_eq!(
        results[0]["externalIds"],
        json!([
            { "source": "smg", "value": "202" },
            { "source": "tmdb", "value": "2020" },
        ])
    );
}

#[tokio::test]
async fn graphql_search_metadata_omits_proxy_for_missing_poster() {
    let ctx = TestContext::new().await;
    let mut fixture: serde_json::Value =
        serde_json::from_str(&load_fixture("smg/search_titles_tmdb_primary.json"))
            .expect("metadata search fixture should be valid JSON");
    fixture["data"]["searchTitles"]["results"][0]["poster_url"] =
        serde_json::Value::String(String::new());
    let fixture =
        serde_json::to_string(&fixture).expect("metadata search fixture should serialize");
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let body = gql(
        &ctx,
        r#"query($query: String!, $type: MediaFacetValue!) {
            searchMetadata(query: $query, type: $type) { posterUrl }
        }"#,
        json!({ "query": "TMDB Primary", "type": "MOVIE" }),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["searchMetadata"][0]["posterUrl"].is_null());

    let source_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM image_proxy_sources")
        .fetch_one(ctx.db.pool())
        .await
        .expect("metadata search should inspect registered image sources");
    assert_eq!(
        source_count, 0,
        "missing artwork must not register a proxy source"
    );
}

#[tokio::test]
async fn graphql_metadata_movie() {
    let ctx = TestContext::new().await;
    let fixture = load_fixture("smg/titles_movie.json");
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let body = gql(
        &ctx,
        r#"query($input: MetadataMovieInput!) {
            metadataMovie(input: $input) {
                smgId tmdbId name year runtimeMinutes overview
            }
        }"#,
        json!({ "input": { "smgId": 101 } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["metadataMovie"]["name"], "Test Movie Title");
    assert_eq!(body["data"]["metadataMovie"]["year"], 2024);
    assert_eq!(body["data"]["metadataMovie"]["runtimeMinutes"], 142);
    assert_eq!(body["data"]["metadataMovie"]["smgId"], 101);
    assert_eq!(body["data"]["metadataMovie"]["tmdbId"], 111);
}

#[tokio::test]
async fn graphql_metadata_series() {
    let ctx = TestContext::new().await;
    // A TVDB-backed series is served through SMG's title surface: the TVDB id
    // resolves to an SMG title id, which `titles` answers.
    let legacy: Value =
        serde_json::from_str(&load_fixture("smg/get_series.json")).expect("fixture is JSON");
    let title_surface = json!({
        "data": { "metadataBulk": { "movies": [], "series": [legacy["data"]["series"]["series"].clone()] } }
    })
    .to_string();
    mount_series_title_surface(&ctx, 8_701, &title_surface).await;
    let fixture = load_fixture("smg/get_series.json");
    Mock::given(method("GET"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
        .mount(&ctx.smg_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture))
        .mount(&ctx.smg_server)
        .await;

    let body = gql(
        &ctx,
        r#"query($input: MetadataSeriesInput!) {
            metadataSeries(input: $input) {
                name year seasons { number label } episodes { name seasonNumber imageUrl }
            }
        }"#,
        json!({ "input": { "tvdbId": "345678" } }),
    )
    .await;
    assert_no_errors(&body);
    let series = &body["data"]["metadataSeries"];
    assert_eq!(series["name"], "Test Show Name");
    assert_eq!(series["seasons"].as_array().unwrap().len(), 2);
    assert_eq!(series["episodes"].as_array().unwrap().len(), 3);
    let image_url = series["episodes"][0]["imageUrl"]
        .as_str()
        .expect("metadata episode image URL should be a string");
    let token = image_url
        .strip_prefix("/images/media/")
        .and_then(|value| value.strip_suffix("/w300"))
        .expect("metadata episode image URL should use Scryer's media route");
    assert_eq!(token.len(), 64);
    assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert!(!image_url.contains("image.tmdb.org"));

    let persisted: (Option<String>, String, String) = sqlx::query_as(
        "SELECT upstream_url, image_kind, fallback_class
           FROM image_proxy_sources
          WHERE token = ?",
    )
    .bind(token)
    .fetch_one(ctx.db.pool())
    .await
    .expect("HTTP GraphQL response should durably register its image source");
    assert_eq!(
        persisted,
        (
            Some("https://image.tmdb.org/t/p/original/pilot.jpg".to_string()),
            "episode_still".to_string(),
            "landscape".to_string(),
        )
    );
}

#[tokio::test]
async fn graphql_metadata_series_by_smg_id_serves_a_tmdb_primary_series() {
    let ctx = TestContext::new().await;
    let fixture = load_fixture("smg/titles_movie.json");
    for http_method in ["GET", "POST"] {
        Mock::given(method(http_method))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
            .mount(&ctx.smg_server)
            .await;
    }

    let body = gql(
        &ctx,
        r#"query($input: MetadataSeriesInput!) {
            metadataSeries(input: $input) {
                tvdbId smgId tmdbId name
                seasons { number }
                episodes { tvdbId tmdbId seasonNumber episodeNumber name }
            }
        }"#,
        json!({ "input": { "smgId": 303 } }),
    )
    .await;
    assert_no_errors(&body);
    let series = &body["data"]["metadataSeries"];
    assert_eq!(series["name"], "TMDB Primary Series");
    assert_eq!(series["tvdbId"], "");
    assert_eq!(series["smgId"], 303);
    assert_eq!(series["tmdbId"], 3030);
    assert_eq!(series["seasons"].as_array().map(Vec::len), Some(1));
    assert_eq!(series["episodes"][0]["tvdbId"], "");
    assert_eq!(series["episodes"][0]["tmdbId"], 3_030_101);
    assert_eq!(series["episodes"][0]["name"], "Pilot");
}

#[tokio::test]
async fn graphql_metadata_series_serves_blank_provider_dates_as_null() {
    let ctx = TestContext::new().await;
    // A provider can leave a series premiere or an episode air date blank.
    // The series still loads, and each missing date reads as null.
    let mut fixture: Value =
        serde_json::from_str(&load_fixture("smg/titles_movie.json")).expect("fixture is JSON");
    let series = &mut fixture["data"]["titles"]["series"][0];
    series["first_aired"] = json!("");
    series["episodes"][0]["aired"] = json!("");
    let fixture = fixture.to_string();
    for http_method in ["GET", "POST"] {
        Mock::given(method(http_method))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_string(fixture.clone()))
            .mount(&ctx.smg_server)
            .await;
    }

    let body = gql(
        &ctx,
        r#"query($input: MetadataSeriesInput!) {
            metadataSeries(input: $input) {
                name firstAired
                episodes { name aired }
            }
        }"#,
        json!({ "input": { "smgId": 303 } }),
    )
    .await;
    assert_no_errors(&body);
    let series = &body["data"]["metadataSeries"];
    assert_eq!(series["name"], "TMDB Primary Series");
    assert_eq!(series["firstAired"], Value::Null);
    assert_eq!(series["episodes"][0]["name"], "Pilot");
    assert_eq!(series["episodes"][0]["aired"], Value::Null);
}

#[tokio::test]
async fn graphql_metadata_series_requires_an_identity() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"query($input: MetadataSeriesInput!) {
            metadataSeries(input: $input) { name }
        }"#,
        json!({ "input": {} }),
    )
    .await;
    let errors = body["errors"].as_array().expect("an identity is required");
    assert!(
        errors[0]["message"]
            .as_str()
            .is_some_and(|message| message.contains("a title identity is required")),
        "{body}"
    );
}
