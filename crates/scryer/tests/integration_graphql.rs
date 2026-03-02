mod common;

use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use common::{load_fixture, TestContext};

/// Helper to execute a GraphQL query and return the parsed JSON body.
async fn gql(ctx: &TestContext, query: &str, variables: Value) -> Value {
    let client = ctx.http_client();
    let resp = client
        .post(ctx.graphql_url())
        .json(&json!({ "query": query, "variables": variables }))
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(resp.status(), 200);
    resp.json().await.expect("should be valid JSON")
}

/// Assert no GraphQL errors in response body.
fn assert_no_errors(body: &Value) {
    assert!(
        body.get("errors").is_none(),
        "unexpected GraphQL errors: {body}"
    );
}

/// Helper to add a title and return the title ID.
async fn add_test_title(ctx: &TestContext, name: &str, facet: &str) -> String {
    let body = gql(
        ctx,
        r#"mutation($input: AddTitleInput!) { addTitle(input: $input) { title { id name } } }"#,
        json!({
            "input": {
                "name": name,
                "facet": facet,
                "monitored": true,
                "tags": [],
                "externalIds": [{ "source": "tvdb", "value": "999" }]
            }
        }),
    )
    .await;
    assert_no_errors(&body);
    body["data"]["addTitle"]["title"]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn mount_smg_mocks(ctx: &TestContext, fixture_path: &str) {
    let fixture = load_fixture(fixture_path);
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
}

// ---------------------------------------------------------------------------
// Basic connectivity
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_get_returns_non_500() {
    let ctx = TestContext::new().await;
    let resp = ctx
        .http_client()
        .get(format!("{}/graphql", ctx.app_url))
        .send()
        .await
        .unwrap();
    // GET on a POST-only endpoint — should not crash
    assert_ne!(resp.status().as_u16(), 500);
}

// ---------------------------------------------------------------------------
// Introspection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_introspection_query_type() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ __schema { queryType { name } } }", json!({})).await;
    assert_eq!(body["data"]["__schema"]["queryType"]["name"], "QueryRoot");
}

#[tokio::test]
async fn graphql_introspection_mutation_type() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ __schema { mutationType { name } } }", json!({})).await;
    assert_eq!(
        body["data"]["__schema"]["mutationType"]["name"],
        "MutationRoot"
    );
}

#[tokio::test]
async fn graphql_introspection_lists_title_fields() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"{ __type(name: "TitlePayload") { fields { name } } }"#,
        json!({}),
    )
    .await;
    let fields = body["data"]["__type"]["fields"]
        .as_array()
        .expect("should have fields");
    let names: Vec<&str> = fields
        .iter()
        .filter_map(|f| f["name"].as_str())
        .collect();
    assert!(names.contains(&"id"), "TitlePayload should have id field");
    assert!(
        names.contains(&"name"),
        "TitlePayload should have name field"
    );
    assert!(
        names.contains(&"facet"),
        "TitlePayload should have facet field"
    );
}

// ---------------------------------------------------------------------------
// Title CRUD
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_list_titles_starts_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ titles { id } }", json!({})).await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["titles"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn graphql_add_title_movie() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Test Movie", "movie").await;
    assert!(!id.is_empty());
}

#[tokio::test]
async fn graphql_add_title_tv() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Test Series", "tv").await;
    assert!(!id.is_empty());
}

#[tokio::test]
async fn graphql_add_title_anime() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Test Anime", "anime").await;
    assert!(!id.is_empty());
}

#[tokio::test]
async fn graphql_add_title_then_list() {
    let ctx = TestContext::new().await;
    add_test_title(&ctx, "Listed Movie", "movie").await;

    let body = gql(&ctx, "{ titles { id name facet } }", json!({})).await;
    assert_no_errors(&body);
    let titles = body["data"]["titles"].as_array().unwrap();
    assert_eq!(titles.len(), 1);
    assert_eq!(titles[0]["name"], "Listed Movie");
    assert_eq!(titles[0]["facet"], "movie");
}

#[tokio::test]
async fn graphql_add_multiple_titles() {
    let ctx = TestContext::new().await;
    add_test_title(&ctx, "Movie One", "movie").await;
    add_test_title(&ctx, "Series One", "tv").await;
    add_test_title(&ctx, "Anime One", "anime").await;

    let body = gql(&ctx, "{ titles { id facet } }", json!({})).await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["titles"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn graphql_get_title_by_id() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Specific Movie", "movie").await;

    let body = gql(
        &ctx,
        r#"query($id: String!) { title(id: $id) { id name monitored } }"#,
        json!({ "id": id }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["title"]["name"], "Specific Movie");
    assert_eq!(body["data"]["title"]["monitored"], true);
}

#[tokio::test]
async fn graphql_get_title_not_found() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"query($id: String!) { title(id: $id) { id name } }"#,
        json!({ "id": "nonexistent-id" }),
    )
    .await;
    assert!(
        body["data"]["title"].is_null(),
        "should return null for nonexistent title"
    );
}

#[tokio::test]
async fn graphql_set_title_monitored() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "Monitor Test", "movie").await;

    // Disable monitoring
    let body = gql(
        &ctx,
        r#"mutation($input: SetTitleMonitoredInput!) {
            setTitleMonitored(input: $input) { id monitored }
        }"#,
        json!({ "input": { "titleId": id, "monitored": false } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["setTitleMonitored"]["monitored"], false);

    // Verify via query
    let body = gql(
        &ctx,
        r#"query($id: String!) { title(id: $id) { monitored } }"#,
        json!({ "id": id }),
    )
    .await;
    assert_eq!(body["data"]["title"]["monitored"], false);
}

#[tokio::test]
async fn graphql_delete_title() {
    let ctx = TestContext::new().await;
    let id = add_test_title(&ctx, "To Delete", "movie").await;

    let body = gql(
        &ctx,
        r#"mutation($input: DeleteTitleInput!) { deleteTitle(input: $input) }"#,
        json!({ "input": { "titleId": id } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["deleteTitle"], true);

    // Verify deleted
    let body = gql(
        &ctx,
        r#"query($id: String!) { title(id: $id) { id } }"#,
        json!({ "id": id }),
    )
    .await;
    assert!(body["data"]["title"].is_null(), "title should be gone");
}

#[tokio::test]
async fn graphql_filter_titles_by_facet() {
    let ctx = TestContext::new().await;
    add_test_title(&ctx, "Movie A", "movie").await;
    add_test_title(&ctx, "Series A", "tv").await;

    let body = gql(
        &ctx,
        r#"query($facet: String) { titles(facet: $facet) { name facet } }"#,
        json!({ "facet": "movie" }),
    )
    .await;
    assert_no_errors(&body);
    let titles = body["data"]["titles"].as_array().unwrap();
    assert_eq!(titles.len(), 1);
    assert_eq!(titles[0]["facet"], "movie");
}

// ---------------------------------------------------------------------------
// User management
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_me_query() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ me { id username } }", json!({})).await;
    assert_no_errors(&body);
    // dev_auto_login creates an "admin" user
    assert_eq!(body["data"]["me"]["username"], "admin");
}

#[tokio::test]
async fn graphql_users_query() {
    let ctx = TestContext::new().await;
    // Trigger dev_auto_login user creation first
    gql(&ctx, "{ me { id } }", json!({})).await;

    let body = gql(&ctx, "{ users { id username } }", json!({})).await;
    assert_no_errors(&body);
    let users = body["data"]["users"].as_array().unwrap();
    assert!(!users.is_empty(), "should have at least one user");
}

#[tokio::test]
async fn graphql_create_user() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation($input: CreateUserInput!) {
            createUser(input: $input) { id username }
        }"#,
        json!({ "input": { "username": "testuser", "password": "testpass123", "entitlements": [] } }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["createUser"]["username"], "testuser");
}

#[tokio::test]
async fn graphql_dev_auto_login() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation { devAutoLogin { token user { username } } }"#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert!(
        body["data"]["devAutoLogin"]["token"].is_string(),
        "should return token"
    );
    assert_eq!(
        body["data"]["devAutoLogin"]["user"]["username"],
        "admin"
    );
}

// ---------------------------------------------------------------------------
// Download queue
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_download_queue_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ downloadQueue { id titleName } }", json!({})).await;
    assert_no_errors(&body);
    let queue = body["data"]["downloadQueue"].as_array().unwrap();
    assert!(queue.is_empty(), "queue should start empty");
}

// ---------------------------------------------------------------------------
// System health
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_system_health() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ systemHealth { serviceReady totalTitles } }", json!({})).await;
    assert_no_errors(&body);
    assert!(
        body["data"]["systemHealth"]["serviceReady"].is_boolean(),
        "should return serviceReady boolean"
    );
}

// ---------------------------------------------------------------------------
// Activity / events
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_activity_events_empty() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        "{ activityEvents { id kind severity } }",
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["activityEvents"].is_array());
}

#[tokio::test]
async fn graphql_title_events_empty() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"{ titleEvents { id eventType } }"#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["titleEvents"].is_array());
}

// ---------------------------------------------------------------------------
// Metadata queries (via SMG mock)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_search_metadata_movie() {
    let ctx = TestContext::new().await;
    let fixture = load_fixture("smg/search_tvdb_rich.json");
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
        r#"query($query: String!, $type: String!) {
            searchMetadata(query: $query, type: $type) {
                tvdbId name year type overview posterUrl
            }
        }"#,
        json!({ "query": "Test Movie", "type": "movie" }),
    )
    .await;
    assert_no_errors(&body);
    let results = body["data"]["searchMetadata"].as_array().unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0]["name"], "Test Movie Title");
}

#[tokio::test]
async fn graphql_metadata_movie() {
    let ctx = TestContext::new().await;
    let fixture = load_fixture("smg/get_movie.json");
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
        r#"query($tvdbId: Int!) {
            metadataMovie(tvdbId: $tvdbId) {
                name year runtimeMinutes genres overview
            }
        }"#,
        json!({ "tvdbId": 123456 }),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["metadataMovie"]["name"], "Test Movie Title");
    assert_eq!(body["data"]["metadataMovie"]["year"], 2024);
    assert_eq!(body["data"]["metadataMovie"]["runtimeMinutes"], 142);
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
        r#"query($id: String!) {
            metadataSeries(id: $id) {
                name year seasons { number label } episodes { name seasonNumber }
            }
        }"#,
        json!({ "id": "345678" }),
    )
    .await;
    assert_no_errors(&body);
    let series = &body["data"]["metadataSeries"];
    assert_eq!(series["name"], "Test Show Name");
    assert_eq!(series["seasons"].as_array().unwrap().len(), 2);
    assert_eq!(series["episodes"].as_array().unwrap().len(), 3);
}

// ---------------------------------------------------------------------------
// Configuration (indexers + download clients)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_indexers_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ indexers { id name } }", json!({})).await;
    assert_no_errors(&body);
    assert!(body["data"]["indexers"].is_array());
}

#[tokio::test]
async fn graphql_download_client_configs_empty() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        "{ downloadClientConfigs { id name } }",
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["downloadClientConfigs"].is_array());
}

// ---------------------------------------------------------------------------
// Wanted items
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_wanted_items_empty() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        "{ wantedItems { items { id } total } }",
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert_eq!(
        body["data"]["wantedItems"]["total"], 0,
        "should have no wanted items initially"
    );
}

// ---------------------------------------------------------------------------
// Rule sets
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_rule_sets_empty() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ ruleSets { id name } }", json!({})).await;
    assert_no_errors(&body);
    assert_eq!(body["data"]["ruleSets"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// Import history
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_import_history_empty() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        "{ importHistory { id sourceTitle status } }",
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["importHistory"].is_array());
}

// ---------------------------------------------------------------------------
// Calendar
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_calendar_episodes() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"query($start: String!, $end: String!) {
            calendarEpisodes(startDate: $start, endDate: $end) {
                episodeTitle seasonNumber episodeNumber
            }
        }"#,
        json!({ "start": "2024-01-01", "end": "2024-12-31" }),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["calendarEpisodes"].is_array());
}

// ---------------------------------------------------------------------------
// Error cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_unknown_field_returns_error() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ nonExistentField }", json!({})).await;
    assert!(
        body.get("errors").is_some(),
        "unknown field should return errors"
    );
}

#[tokio::test]
async fn graphql_invalid_mutation_input() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"mutation { addTitle(input: { name: "" }) { title { id } } }"#,
        json!({}),
    )
    .await;
    assert!(
        body.get("errors").is_some(),
        "invalid input should return errors"
    );
}

#[tokio::test]
async fn graphql_batch_request_not_supported_via_single() {
    let ctx = TestContext::new().await;
    // Verify single requests work (batch is handled at the middleware level)
    let body = gql(&ctx, "{ titles { id } }", json!({})).await;
    assert_no_errors(&body);
}
