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
    assert_eq!(
        results[0]["externalIds"],
        json!([
            { "source": "smg", "value": "202" },
            { "source": "tmdb", "value": "2020" },
        ])
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
