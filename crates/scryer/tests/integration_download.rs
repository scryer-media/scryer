mod common;

use serde_json::json;
use wiremock::matchers::{body_json_string, method, path};
use wiremock::{Mock, ResponseTemplate};

use common::{load_fixture, TestContext};
use scryer_application::DownloadClient;

fn new_nzbget_client(uri: &str) -> scryer_infrastructure::NzbgetDownloadClient {
    scryer_infrastructure::NzbgetDownloadClient::new(
        uri.to_string(),
        Some("test-user".to_string()),
        Some("test-pass".to_string()),
        "SCORE".to_string(),
    )
}

// ---------------------------------------------------------------------------
// test_connection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nzbget_test_connection_returns_version() {
    let ctx = TestContext::new().await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(ResponseTemplate::new(200).set_body_string(load_fixture("nzbget/version.json")))
        .mount(&ctx.nzbget_server)
        .await;

    let result = new_nzbget_client(&ctx.nzbget_server.uri())
        .test_connection()
        .await;
    assert_eq!(result.unwrap(), "24.3");
}

#[tokio::test]
async fn nzbget_test_connection_unreachable() {
    let client = scryer_infrastructure::NzbgetDownloadClient::new(
        "http://127.0.0.1:1".to_string(),
        None,
        None,
        "SCORE".to_string(),
    );
    let result = client.test_connection().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn nzbget_test_connection_http_500() {
    let ctx = TestContext::new().await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&ctx.nzbget_server)
        .await;

    let result = new_nzbget_client(&ctx.nzbget_server.uri())
        .test_connection()
        .await;
    assert!(result.is_err(), "should fail on HTTP 500");
    assert!(
        result.unwrap_err().to_string().contains("500"),
        "error should mention status code"
    );
}

#[tokio::test]
async fn nzbget_test_connection_rpc_error() {
    let ctx = TestContext::new().await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbget/rpc_error.json")),
        )
        .mount(&ctx.nzbget_server)
        .await;

    let result = new_nzbget_client(&ctx.nzbget_server.uri())
        .test_connection()
        .await;
    assert!(result.is_err(), "should fail on JSON-RPC error");
    assert!(
        result.unwrap_err().to_string().contains("Method not found"),
        "error should contain RPC message"
    );
}

#[tokio::test]
async fn nzbget_test_connection_invalid_json() {
    let ctx = TestContext::new().await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json at all"))
        .mount(&ctx.nzbget_server)
        .await;

    let result = new_nzbget_client(&ctx.nzbget_server.uri())
        .test_connection()
        .await;
    assert!(result.is_err(), "should fail on invalid JSON");
}

// ---------------------------------------------------------------------------
// list_queue
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nzbget_list_queue_two_items() {
    let ctx = TestContext::new().await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_json_string(
            r#"{"version":"2.0","method":"listgroups","params":[],"id":"scryer-rpc"}"#,
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbget/listgroups.json")),
        )
        .mount(&ctx.nzbget_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_json_string(
            r#"{"version":"2.0","method":"postqueue","params":[],"id":"scryer-rpc"}"#,
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbget/postqueue.json")),
        )
        .mount(&ctx.nzbget_server)
        .await;

    let items = new_nzbget_client(&ctx.nzbget_server.uri())
        .list_queue()
        .await
        .expect("list_queue should succeed");
    assert_eq!(items.len(), 2);
}

#[tokio::test]
async fn nzbget_list_queue_empty() {
    let ctx = TestContext::new().await;
    // Return empty arrays for both listgroups and postqueue
    let empty_groups = json!({"version":"2.0","id":"scryer-rpc","result":[]});
    let empty_post = json!({"version":"2.0","id":"scryer-rpc","result":[]});

    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_json_string(
            r#"{"version":"2.0","method":"listgroups","params":[],"id":"scryer-rpc"}"#,
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(&empty_groups))
        .mount(&ctx.nzbget_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_json_string(
            r#"{"version":"2.0","method":"postqueue","params":[],"id":"scryer-rpc"}"#,
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(&empty_post))
        .mount(&ctx.nzbget_server)
        .await;

    let items = new_nzbget_client(&ctx.nzbget_server.uri())
        .list_queue()
        .await
        .expect("empty queue should succeed");
    assert!(items.is_empty());
}

#[tokio::test]
async fn nzbget_list_queue_item_has_correct_fields() {
    let ctx = TestContext::new().await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_json_string(
            r#"{"version":"2.0","method":"listgroups","params":[],"id":"scryer-rpc"}"#,
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbget/listgroups.json")),
        )
        .mount(&ctx.nzbget_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_json_string(
            r#"{"version":"2.0","method":"postqueue","params":[],"id":"scryer-rpc"}"#,
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbget/postqueue.json")),
        )
        .mount(&ctx.nzbget_server)
        .await;

    let items = new_nzbget_client(&ctx.nzbget_server.uri())
        .list_queue()
        .await
        .unwrap();

    let first = &items[0];
    assert!(!first.title_name.is_empty(), "title_name should be set");
    assert!(first.size_bytes.is_some(), "size should be set");
}

// ---------------------------------------------------------------------------
// list_history
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nzbget_list_history_filters_old_entries() {
    let ctx = TestContext::new().await;
    // Use original fixture with old timestamps — should filter out everything
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbget/history.json")),
        )
        .mount(&ctx.nzbget_server)
        .await;

    let items = new_nzbget_client(&ctx.nzbget_server.uri())
        .list_history()
        .await
        .expect("list_history should succeed even with old entries");
    assert!(
        items.is_empty(),
        "old entries beyond 7-day cutoff should be filtered out"
    );
}

#[tokio::test]
async fn nzbget_list_history_recent_entries() {
    let ctx = TestContext::new().await;
    let now = chrono::Utc::now().timestamp();
    let history = load_fixture("nzbget/history.json")
        .replace("1706832000", &now.to_string())
        .replace("1706745600", &(now - 3600).to_string());

    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(ResponseTemplate::new(200).set_body_string(history))
        .mount(&ctx.nzbget_server)
        .await;

    let items = new_nzbget_client(&ctx.nzbget_server.uri())
        .list_history()
        .await
        .unwrap();
    assert_eq!(items.len(), 2, "recent entries should pass 7-day cutoff");
}

#[tokio::test]
async fn nzbget_list_history_maps_success_status() {
    let ctx = TestContext::new().await;
    let now = chrono::Utc::now().timestamp();
    let history = load_fixture("nzbget/history.json")
        .replace("1706832000", &now.to_string())
        .replace("1706745600", &(now - 3600).to_string());

    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(ResponseTemplate::new(200).set_body_string(history))
        .mount(&ctx.nzbget_server)
        .await;

    let items = new_nzbget_client(&ctx.nzbget_server.uri())
        .list_history()
        .await
        .unwrap();

    // First item has SUCCESS/ALL status
    let success_item = items.iter().find(|i| i.title_name.contains("Completed")).unwrap();
    assert_eq!(
        format!("{:?}", success_item.state),
        "Completed",
        "SUCCESS should map to Completed"
    );

    // Second item has FAILURE/HEALTH status
    let failed_item = items.iter().find(|i| i.title_name.contains("Failed")).unwrap();
    assert_eq!(
        format!("{:?}", failed_item.state),
        "Failed",
        "FAILURE should map to Failed"
    );
}

#[tokio::test]
async fn nzbget_list_history_empty() {
    let ctx = TestContext::new().await;
    let empty = json!({"version":"2.0","id":"scryer-rpc","result":[]});
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&empty))
        .mount(&ctx.nzbget_server)
        .await;

    let items = new_nzbget_client(&ctx.nzbget_server.uri())
        .list_history()
        .await
        .unwrap();
    assert!(items.is_empty());
}

// ---------------------------------------------------------------------------
// pause / resume / delete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nzbget_pause_queue_item() {
    let ctx = TestContext::new().await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(load_fixture("nzbget/editqueue_success.json")),
        )
        .mount(&ctx.nzbget_server)
        .await;

    let result = new_nzbget_client(&ctx.nzbget_server.uri())
        .pause_queue_item("12345")
        .await;
    assert!(result.is_ok(), "pause should succeed: {:?}", result.err());
}

#[tokio::test]
async fn nzbget_resume_queue_item() {
    let ctx = TestContext::new().await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(load_fixture("nzbget/editqueue_success.json")),
        )
        .mount(&ctx.nzbget_server)
        .await;

    let result = new_nzbget_client(&ctx.nzbget_server.uri())
        .resume_queue_item("12345")
        .await;
    assert!(result.is_ok(), "resume should succeed: {:?}", result.err());
}

#[tokio::test]
async fn nzbget_delete_queue_item() {
    let ctx = TestContext::new().await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(load_fixture("nzbget/editqueue_success.json")),
        )
        .mount(&ctx.nzbget_server)
        .await;

    let result = new_nzbget_client(&ctx.nzbget_server.uri())
        .delete_queue_item("12345", false)
        .await;
    assert!(result.is_ok(), "delete should succeed: {:?}", result.err());
}

#[tokio::test]
async fn nzbget_delete_history_item() {
    let ctx = TestContext::new().await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(load_fixture("nzbget/editqueue_success.json")),
        )
        .mount(&ctx.nzbget_server)
        .await;

    let result = new_nzbget_client(&ctx.nzbget_server.uri())
        .delete_queue_item("999", true)
        .await;
    assert!(
        result.is_ok(),
        "history delete should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn nzbget_pause_invalid_id() {
    let ctx = TestContext::new().await;
    // No mock needed — should fail parsing "not-a-number" to i64
    let result = new_nzbget_client(&ctx.nzbget_server.uri())
        .pause_queue_item("not-a-number")
        .await;
    assert!(result.is_err(), "non-numeric ID should fail");
}

// ---------------------------------------------------------------------------
// list_completed_downloads
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nzbget_list_completed_downloads() {
    let ctx = TestContext::new().await;
    let now = chrono::Utc::now().timestamp();
    let history = load_fixture("nzbget/history.json")
        .replace("1706832000", &now.to_string())
        .replace("1706745600", &(now - 3600).to_string());

    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(ResponseTemplate::new(200).set_body_string(history))
        .mount(&ctx.nzbget_server)
        .await;

    let items = new_nzbget_client(&ctx.nzbget_server.uri())
        .list_completed_downloads()
        .await
        .expect("list_completed_downloads should succeed");

    // Only SUCCESS items should be returned
    assert_eq!(
        items.len(),
        1,
        "should return only SUCCESS entries, not FAILURE"
    );
    assert!(items[0].dest_dir.contains("Completed"));
}

// ---------------------------------------------------------------------------
// submit_to_download_queue
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nzbget_submit_download() {
    let ctx = TestContext::new().await;
    let nzb_xml = load_fixture("nzbgeek/nzb_content.xml");

    // Mock the NZB download URL (fetch_and_encode_nzb fetches from source_hint)
    Mock::given(method("GET"))
        .and(path("/getnzb/test.nzb"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(nzb_xml)
                .insert_header("content-type", "application/x-nzb"),
        )
        .mount(&ctx.nzbget_server)
        .await;

    // Mock the NZBGet append RPC
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbget/append.json")),
        )
        .mount(&ctx.nzbget_server)
        .await;

    let title = scryer_domain::Title {
        id: "test-title-id".to_string(),
        name: "Test Movie Title".to_string(),
        facet: scryer_domain::MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2024),
        overview: None,
        poster_url: None,
        sort_title: None,
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        genres: vec![],
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
    };

    let source_hint = format!("{}/getnzb/test.nzb", ctx.nzbget_server.uri());
    let result = new_nzbget_client(&ctx.nzbget_server.uri())
        .submit_to_download_queue(&title, Some(source_hint), None, None, None)
        .await;

    assert!(
        result.is_ok(),
        "submit should succeed: {:?}",
        result.err()
    );
    assert!(
        !result.unwrap().is_empty(),
        "should return a non-empty job ID"
    );
}

#[tokio::test]
async fn nzbget_submit_download_no_source_hint() {
    let ctx = TestContext::new().await;
    let title = scryer_domain::Title {
        id: "test-id".to_string(),
        name: "Test".to_string(),
        facet: scryer_domain::MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: chrono::Utc::now(),
        year: None,
        overview: None,
        poster_url: None,
        sort_title: None,
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        genres: vec![],
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
    };

    let result = new_nzbget_client(&ctx.nzbget_server.uri())
        .submit_to_download_queue(&title, None, None, None, None)
        .await;
    assert!(result.is_err(), "should fail without source_hint");
}

// ---------------------------------------------------------------------------
// endpoint construction
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nzbget_endpoint_appends_jsonrpc() {
    let client = scryer_infrastructure::NzbgetDownloadClient::new(
        "http://localhost:6789".to_string(),
        None,
        None,
        "SCORE".to_string(),
    );
    assert_eq!(client.endpoint(), "http://localhost:6789/jsonrpc");
}

#[tokio::test]
async fn nzbget_endpoint_preserves_existing_jsonrpc() {
    let client = scryer_infrastructure::NzbgetDownloadClient::new(
        "http://localhost:6789/jsonrpc".to_string(),
        None,
        None,
        "SCORE".to_string(),
    );
    assert_eq!(client.endpoint(), "http://localhost:6789/jsonrpc");
}

#[tokio::test]
async fn nzbget_endpoint_strips_trailing_slash() {
    let client = scryer_infrastructure::NzbgetDownloadClient::new(
        "http://localhost:6789/".to_string(),
        None,
        None,
        "SCORE".to_string(),
    );
    assert_eq!(client.endpoint(), "http://localhost:6789/jsonrpc");
}
