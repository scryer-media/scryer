#![recursion_limit = "256"]

mod common;

use std::sync::Arc;

use serde_json::json;
use tokio::sync::Semaphore;
use wiremock::matchers::{body_json_string, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestContext, load_fixture};
use scryer_application::{
    DownloadClient, DownloadClientAddRequest, DownloadClientConfigRepository, DownloadSourceKind,
    NullSettingsRepository, NullStagedNzbStore, StagedNzbRef,
};
use scryer_domain::DownloadClientConfig;
use scryer_infrastructure::{
    FileSystemStagedNzbStore, NzbgetDownloadClient, PrioritizedDownloadClientRouter,
    SabnzbdDownloadClient, WeaverDownloadClient,
};

fn new_nzbget_client(uri: &str) -> scryer_infrastructure::NzbgetDownloadClient {
    scryer_infrastructure::NzbgetDownloadClient::new(
        uri.to_string(),
        Some("test-user".to_string()),
        Some("test-pass".to_string()),
        "SCORE".to_string(),
    )
}

async fn new_submit_nzbget_client(uri: &str) -> scryer_infrastructure::NzbgetDownloadClient {
    scryer_infrastructure::NzbgetDownloadClient::with_staged_nzb_store(
        uri.to_string(),
        Some("test-user".to_string()),
        Some("test-pass".to_string()),
        "SCORE".to_string(),
        new_staged_nzb_store().await,
        Arc::new(Semaphore::new(4)),
    )
}

async fn new_staged_nzb_store() -> Arc<FileSystemStagedNzbStore> {
    let dir = std::env::temp_dir().join(format!(
        "scryer-test-staged-nzb-{}",
        scryer_domain::Id::new().0
    ));
    Arc::new(
        FileSystemStagedNzbStore::new(&dir)
            .await
            .expect("staged nzb store"),
    )
}

fn test_title(name: &str) -> scryer_domain::Title {
    scryer_domain::Title {
        id: format!("title-{}", name.replace(' ', "-").to_ascii_lowercase()),
        name: name.to_string(),
        facet: scryer_domain::MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2024),
        overview: None,
        poster_url: None,
        poster_source_url: None,
        banner_url: None,
        banner_source_url: None,
        background_url: None,
        background_source_url: None,
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
        tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    }
}

fn request_with_staged_nzb(
    title: scryer_domain::Title,
    staged_nzb: StagedNzbRef,
    source_title: &str,
) -> DownloadClientAddRequest {
    DownloadClientAddRequest {
        title,
        source_hint: None,
        staged_nzb: Some(staged_nzb),
        source_kind: Some(DownloadSourceKind::NzbFile),
        source_title: Some(source_title.to_string()),
        source_password: None,
        category: Some("movies".to_string()),
        queue_priority: None,
        download_directory: None,
        release_title: None,
        indexer_name: None,
        info_hash_hint: None,
        seed_goal_ratio: None,
        seed_goal_seconds: None,
        is_recent: None,
        season_pack: None,
    }
}

// ---------------------------------------------------------------------------
// test_connection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nzbget_test_connection_returns_version() {
    let ctx = TestContext::new().await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbget/version.json")),
        )
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
    let success_item = items
        .iter()
        .find(|i| i.title_name.contains("Completed"))
        .unwrap();
    assert_eq!(
        format!("{:?}", success_item.state),
        "Completed",
        "SUCCESS should map to Completed"
    );

    // Second item has FAILURE/HEALTH status
    let failed_item = items
        .iter()
        .find(|i| i.title_name.contains("Failed"))
        .unwrap();
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
        poster_source_url: None,
        banner_url: None,
        banner_source_url: None,
        background_url: None,
        background_source_url: None,
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
        tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    };

    let source_hint = format!("{}/getnzb/test.nzb", ctx.nzbget_server.uri());
    let result = new_submit_nzbget_client(&ctx.nzbget_server.uri())
        .await
        .submit_to_download_queue(&title, Some(source_hint), None, None, None, None)
        .await;

    assert!(result.is_ok(), "submit should succeed: {:?}", result.err());
    let grab = result.unwrap();
    assert!(!grab.job_id.is_empty(), "should return a non-empty job ID");
}

#[tokio::test]
async fn nzbget_submit_download_supports_v25_3_append_signature() {
    let ctx = TestContext::new().await;
    let nzb_xml = load_fixture("nzbgeek/nzb_content.xml");

    Mock::given(method("GET"))
        .and(path("/getnzb/test.nzb"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(nzb_xml)
                .insert_header("content-type", "application/x-nzb"),
        )
        .mount(&ctx.nzbget_server)
        .await;

    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_json_string(
            r#"{"version":"2.0","method":"version","params":[],"id":"scryer-rpc"}"#,
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "version": "2.0",
            "id": "scryer-rpc",
            "result": "25.3"
        })))
        .mount(&ctx.nzbget_server)
        .await;

    // Append mock — matches any POST /jsonrpc that doesn't match the
    // version mock above (wiremock tries mocks in reverse registration
    // order, so version's exact-body matcher is checked first).
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
        poster_source_url: None,
        banner_url: None,
        banner_source_url: None,
        background_url: None,
        background_source_url: None,
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
        tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    };

    let source_hint = format!("{}/getnzb/test.nzb", ctx.nzbget_server.uri());
    let result = new_submit_nzbget_client(&ctx.nzbget_server.uri())
        .await
        .submit_to_download_queue(&title, Some(source_hint), None, None, None, None)
        .await;

    assert!(
        result.is_ok(),
        "submit against nzbget 25.3 should succeed: {:?}",
        result.err()
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
        poster_source_url: None,
        banner_url: None,
        banner_source_url: None,
        background_url: None,
        background_source_url: None,
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
        tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    };

    let result = new_submit_nzbget_client(&ctx.nzbget_server.uri())
        .await
        .submit_to_download_queue(&title, None, None, None, None, None)
        .await;
    assert!(result.is_err(), "should fail without source_hint");
}

#[tokio::test]
async fn nzbget_submit_download_deletes_self_staged_nzb_on_failure() {
    let ctx = TestContext::new().await;
    let nzb_xml = load_fixture("nzbgeek/nzb_content.xml");
    let staged_nzb_store = new_staged_nzb_store().await;

    Mock::given(method("GET"))
        .and(path("/getnzb/test.nzb"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(nzb_xml)
                .insert_header("content-type", "application/x-nzb"),
        )
        .mount(&ctx.nzbget_server)
        .await;

    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(ResponseTemplate::new(500).set_body_string("append failed"))
        .mount(&ctx.nzbget_server)
        .await;

    let client = NzbgetDownloadClient::with_staged_nzb_store(
        ctx.nzbget_server.uri(),
        Some("test-user".to_string()),
        Some("test-pass".to_string()),
        "SCORE".to_string(),
        staged_nzb_store.clone(),
        Arc::new(Semaphore::new(4)),
    );

    let error = client
        .submit_to_download_queue(
            &test_title("Broken NZBGet Submit"),
            Some(format!("{}/getnzb/test.nzb", ctx.nzbget_server.uri())),
            Some(DownloadSourceKind::NzbUrl),
            Some("Broken.Release".to_string()),
            None,
            None,
        )
        .await
        .expect_err("submit should fail");

    assert!(error.to_string().contains("500") || error.to_string().contains("failed"));
    assert_eq!(staged_nzb_store.count_staged_artifacts().await.unwrap(), 0);
}

#[tokio::test]
async fn nzbget_submit_download_uses_staged_cache_entry_without_refetch() {
    let server = MockServer::start().await;
    let staged_nzb_store = new_staged_nzb_store().await;
    let nzb_xml = load_fixture("nzbgeek/nzb_content.xml");

    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbget/append.json")),
        )
        .mount(&server)
        .await;

    let staged = staged_nzb_store
        .stage_nzb_bytes_for_test(nzb_xml.as_bytes())
        .await
        .expect("staged artifact should insert");

    let client = NzbgetDownloadClient::with_staged_nzb_store(
        server.uri(),
        Some("test-user".to_string()),
        Some("test-pass".to_string()),
        "SCORE".to_string(),
        staged_nzb_store.clone(),
        Arc::new(Semaphore::new(4)),
    );

    let result = client
        .submit_download(&request_with_staged_nzb(
            test_title("Staged NZBGet"),
            staged,
            "Staged.NZBGet.Release",
        ))
        .await
        .expect("submit should use staged nzb");

    assert!(!result.job_id.is_empty());
    assert_eq!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.method.as_str() == "GET")
            .count(),
        0
    );
    assert_eq!(staged_nzb_store.count_staged_artifacts().await.unwrap(), 1);
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

// ---------------------------------------------------------------------------
// PrioritizedDownloadClientRouter
// ---------------------------------------------------------------------------

/// Build a minimal enabled DownloadClientConfig pointing at `base_url`.
fn router_config(id: &str, base_url: &str, priority: i64, enabled: bool) -> DownloadClientConfig {
    // Extract host:port from base_url for config_json.
    let stripped = base_url
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/');
    let (host, port) = stripped.rsplit_once(':').unwrap_or((stripped, ""));
    let config_json = serde_json::json!({
        "host": host,
        "port": port,
        "use_ssl": base_url.starts_with("https"),
        "username": "scryer",
        "password": "",
        "client_type": "nzbget",
    })
    .to_string();
    DownloadClientConfig {
        id: id.to_string(),
        name: format!("test-{id}"),
        client_type: "nzbget".to_string(),
        config_json,
        client_priority: priority,
        is_enabled: enabled,
        status: scryer_domain::DownloadClientStatus::Healthy,
        last_error: None,
        last_seen_at: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

/// Mount the listgroups + postqueue mocks needed for list_queue() to succeed.
async fn mount_list_queue_mocks(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_json_string(
            r#"{"version":"2.0","method":"listgroups","params":[],"id":"scryer-rpc"}"#,
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbget/listgroups.json")),
        )
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .and(body_json_string(
            r#"{"version":"2.0","method":"postqueue","params":[],"id":"scryer-rpc"}"#,
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbget/postqueue.json")),
        )
        .mount(server)
        .await;
}

/// Create a router backed by the test DB, with `fallback_uri` as the fallback client.
fn build_router(ctx: &TestContext, fallback_uri: String) -> PrioritizedDownloadClientRouter {
    build_router_with_cache(ctx, fallback_uri, Arc::new(NullStagedNzbStore))
}

fn build_router_with_cache(
    ctx: &TestContext,
    fallback_uri: String,
    staged_nzb_store: Arc<dyn scryer_application::StagedNzbStore>,
) -> PrioritizedDownloadClientRouter {
    let fallback = NzbgetDownloadClient::new(fallback_uri, None, None, "SCORE".to_string());
    PrioritizedDownloadClientRouter::new(
        Arc::new(ctx.db.clone()),
        Arc::new(NullSettingsRepository),
        Arc::new(fallback),
        staged_nzb_store,
        Arc::new(Semaphore::new(4)),
        None,
    )
}

#[tokio::test]
async fn router_routes_to_highest_priority_client() {
    let ctx = TestContext::new().await;
    let second_server = MockServer::start().await;

    // Only the priority-1 server is mocked to succeed.
    mount_list_queue_mocks(&ctx.nzbget_server).await;
    // second_server has no mocks — any request there would fail.

    // Insert configs out-of-order to confirm priority ordering beats insertion order.
    ctx.db
        .create(router_config("c2", &second_server.uri(), 2, true))
        .await
        .unwrap();
    ctx.db
        .create(router_config("c1", &ctx.nzbget_server.uri(), 1, true))
        .await
        .unwrap();

    let router = build_router(&ctx, "http://127.0.0.1:1".to_string());
    let items = router
        .list_queue()
        .await
        .expect("priority-1 client should succeed");

    // Aggregation: primary returns 2 items, secondary has no mocks so its
    // request fails and is skipped — total is still 2 from primary.
    assert_eq!(
        items.len(),
        2,
        "should return items from the primary client"
    );
}

#[tokio::test]
async fn router_falls_back_to_next_client_on_primary_failure() {
    let ctx = TestContext::new().await;
    let second_server = MockServer::start().await;

    // Primary (priority 1) has no mocks — wiremock returns 404 for unmatched requests.
    // Secondary (priority 2) is mocked to succeed.
    mount_list_queue_mocks(&second_server).await;

    ctx.db
        .create(router_config("c1", &ctx.nzbget_server.uri(), 1, true))
        .await
        .unwrap();
    ctx.db
        .create(router_config("c2", &second_server.uri(), 2, true))
        .await
        .unwrap();

    let router = build_router(&ctx, "http://127.0.0.1:1".to_string());
    let items = router
        .list_queue()
        .await
        .expect("secondary client should succeed after primary fails");

    assert_eq!(
        items.len(),
        2,
        "should return items from the secondary client"
    );
    assert!(
        !second_server.received_requests().await.unwrap().is_empty(),
        "secondary client should have been contacted"
    );
}

#[tokio::test]
async fn router_uses_fallback_when_no_clients_configured() {
    let ctx = TestContext::new().await;

    // No configs in DB — the fallback client is the only option.
    mount_list_queue_mocks(&ctx.nzbget_server).await;

    // The fallback is pointed at the only mocked server.
    let fallback =
        NzbgetDownloadClient::new(ctx.nzbget_server.uri(), None, None, "SCORE".to_string());
    let router = PrioritizedDownloadClientRouter::new(
        Arc::new(ctx.db.clone()),
        Arc::new(NullSettingsRepository),
        Arc::new(fallback),
        Arc::new(NullStagedNzbStore),
        Arc::new(Semaphore::new(4)),
        None,
    );

    let items = router
        .list_queue()
        .await
        .expect("fallback client should be used when no configs exist");

    assert_eq!(items.len(), 2);
}

#[tokio::test]
async fn router_skips_client_with_invalid_config() {
    let ctx = TestContext::new().await;

    // Priority 1: sabnzbd with missing API key — client_from_config returns Validation error, skipped.
    let bad_config = DownloadClientConfig {
        id: "bad".to_string(),
        name: "bad-client".to_string(),
        client_type: "sabnzbd".to_string(),
        config_json: "{}".to_string(),
        client_priority: 1,
        is_enabled: true,
        status: scryer_domain::DownloadClientStatus::Healthy,
        last_error: None,
        last_seen_at: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    ctx.db.create(bad_config).await.unwrap();

    // Priority 2: valid nzbget client, mocked to succeed.
    let second_server = MockServer::start().await;
    mount_list_queue_mocks(&second_server).await;
    ctx.db
        .create(router_config("good", &second_server.uri(), 2, true))
        .await
        .unwrap();

    let router = build_router(&ctx, "http://127.0.0.1:1".to_string());
    let items = router
        .list_queue()
        .await
        .expect("valid nzbget client should be used after skipping invalid config");

    assert_eq!(items.len(), 2);
}

#[tokio::test]
async fn router_skips_client_missing_base_url() {
    let ctx = TestContext::new().await;

    // Priority 1: no base_url, empty JSON config — resolve_download_client_base_url returns None.
    let no_url_config = DownloadClientConfig {
        id: "no-url".to_string(),
        name: "no-url-client".to_string(),
        client_type: "nzbget".to_string(),
        config_json: "{}".to_string(),
        client_priority: 1,
        is_enabled: true,
        status: scryer_domain::DownloadClientStatus::Healthy,
        last_error: None,
        last_seen_at: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    ctx.db.create(no_url_config).await.unwrap();

    // Priority 2: valid config.
    mount_list_queue_mocks(&ctx.nzbget_server).await;
    ctx.db
        .create(router_config("valid", &ctx.nzbget_server.uri(), 2, true))
        .await
        .unwrap();

    let router = build_router(&ctx, "http://127.0.0.1:1".to_string());
    let items = router
        .list_queue()
        .await
        .expect("valid client should succeed after skipping the no-url client");

    assert_eq!(items.len(), 2);
}

#[tokio::test]
async fn router_disabled_clients_are_not_used() {
    let ctx = TestContext::new().await;

    // Disabled client at priority 1 — should be filtered out.
    ctx.db
        .create(router_config(
            "disabled",
            &ctx.nzbget_server.uri(),
            1,
            false,
        ))
        .await
        .unwrap();

    // No enabled clients → fallback is used.
    let fallback_server = MockServer::start().await;
    mount_list_queue_mocks(&fallback_server).await;
    let fallback =
        NzbgetDownloadClient::new(fallback_server.uri(), None, None, "SCORE".to_string());
    let router = PrioritizedDownloadClientRouter::new(
        Arc::new(ctx.db.clone()),
        Arc::new(NullSettingsRepository),
        Arc::new(fallback),
        Arc::new(NullStagedNzbStore),
        Arc::new(Semaphore::new(4)),
        None,
    );

    let items = router
        .list_queue()
        .await
        .expect("fallback should be used when only client is disabled");

    assert_eq!(items.len(), 2);
    // Disabled client's server received no requests.
    assert!(
        ctx.nzbget_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn router_reuses_single_staged_nzb_across_client_failover() {
    let ctx = TestContext::new().await;
    let source_server = MockServer::start().await;
    let second_client_server = MockServer::start().await;
    let nzb_xml = load_fixture("nzbgeek/nzb_content.xml");
    let staged_nzb_store = new_staged_nzb_store().await;

    Mock::given(method("GET"))
        .and(path("/release.nzb"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(nzb_xml)
                .insert_header("content-type", "application/x-nzb"),
        )
        .mount(&source_server)
        .await;

    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(ResponseTemplate::new(500).set_body_string("append failed"))
        .mount(&ctx.nzbget_server)
        .await;

    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("nzbget/append.json")),
        )
        .mount(&second_client_server)
        .await;

    ctx.db
        .create(router_config("primary", &ctx.nzbget_server.uri(), 1, true))
        .await
        .unwrap();
    ctx.db
        .create(router_config(
            "secondary",
            &second_client_server.uri(),
            2,
            true,
        ))
        .await
        .unwrap();

    let router = build_router_with_cache(
        &ctx,
        "http://127.0.0.1:1".to_string(),
        staged_nzb_store.clone(),
    );
    let result = router
        .submit_to_download_queue(
            &test_title("Router Failover"),
            Some(format!("{}/release.nzb", source_server.uri())),
            Some(DownloadSourceKind::NzbUrl),
            Some("Router.Failover.Release".to_string()),
            None,
            None,
        )
        .await
        .expect("secondary client should succeed after failover");

    assert_eq!(result.client_type, "nzbget");
    assert_eq!(source_server.received_requests().await.unwrap().len(), 1);
    assert_eq!(staged_nzb_store.count_staged_artifacts().await.unwrap(), 0);
}

#[tokio::test]
async fn router_deletes_staged_nzb_after_final_failure() {
    let ctx = TestContext::new().await;
    let source_server = MockServer::start().await;
    let second_client_server = MockServer::start().await;
    let nzb_xml = load_fixture("nzbgeek/nzb_content.xml");
    let staged_nzb_store = new_staged_nzb_store().await;

    Mock::given(method("GET"))
        .and(path("/release.nzb"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(nzb_xml)
                .insert_header("content-type", "application/x-nzb"),
        )
        .mount(&source_server)
        .await;

    for server in [&ctx.nzbget_server, &second_client_server] {
        Mock::given(method("POST"))
            .and(path("/jsonrpc"))
            .respond_with(ResponseTemplate::new(500).set_body_string("append failed"))
            .mount(server)
            .await;
    }

    ctx.db
        .create(router_config("primary", &ctx.nzbget_server.uri(), 1, true))
        .await
        .unwrap();
    ctx.db
        .create(router_config(
            "secondary",
            &second_client_server.uri(),
            2,
            true,
        ))
        .await
        .unwrap();

    let router = build_router_with_cache(
        &ctx,
        "http://127.0.0.1:1".to_string(),
        staged_nzb_store.clone(),
    );
    let error = router
        .submit_to_download_queue(
            &test_title("Router Failure"),
            Some(format!("{}/release.nzb", source_server.uri())),
            Some(DownloadSourceKind::NzbUrl),
            Some("Router.Failure.Release".to_string()),
            None,
            None,
        )
        .await
        .expect_err("all clients should fail");

    assert!(error.to_string().contains("500") || error.to_string().contains("failed"));
    assert_eq!(source_server.received_requests().await.unwrap().len(), 1);
    assert_eq!(staged_nzb_store.count_staged_artifacts().await.unwrap(), 0);
}

// ===========================================================================
// SABnzbd integration tests
// ===========================================================================

fn new_sabnzbd_client(uri: &str) -> SabnzbdDownloadClient {
    SabnzbdDownloadClient::new(uri.to_string(), "test-api-key".to_string())
}

async fn new_submit_sabnzbd_client(uri: &str) -> SabnzbdDownloadClient {
    SabnzbdDownloadClient::with_staged_nzb_store(
        uri.to_string(),
        "test-api-key".to_string(),
        new_staged_nzb_store().await,
        Arc::new(Semaphore::new(4)),
    )
}

// ---------------------------------------------------------------------------
// test_connection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sabnzbd_test_connection_returns_version() {
    let server = MockServer::start().await;

    // Version endpoint (no auth)
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "version"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/version.json")),
        )
        .mount(&server)
        .await;

    // Queue endpoint (validates API key)
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "queue"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/queue_empty.json")),
        )
        .mount(&server)
        .await;

    let result = new_sabnzbd_client(&server.uri()).test_connection().await;
    assert_eq!(result.unwrap(), "4.5.1");
}

#[tokio::test]
async fn sabnzbd_test_connection_unreachable() {
    let client = SabnzbdDownloadClient::new("http://127.0.0.1:1".to_string(), "key".to_string());
    let result = client.test_connection().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn sabnzbd_test_connection_invalid_api_key() {
    let server = MockServer::start().await;

    // Version succeeds (no auth needed)
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "version"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/version.json")),
        )
        .mount(&server)
        .await;

    // Queue returns auth error
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "queue"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/error.json")),
        )
        .mount(&server)
        .await;

    let result = new_sabnzbd_client(&server.uri()).test_connection().await;
    assert!(result.is_err(), "should fail with invalid API key");
    assert!(
        result.unwrap_err().to_string().contains("API Key"),
        "error should mention API key"
    );
}

// ---------------------------------------------------------------------------
// list_queue
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sabnzbd_list_queue_two_items() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "queue"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/queue.json")),
        )
        .mount(&server)
        .await;

    let items = new_sabnzbd_client(&server.uri())
        .list_queue()
        .await
        .expect("list_queue should succeed");
    assert_eq!(items.len(), 2);
}

#[tokio::test]
async fn sabnzbd_list_queue_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "queue"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/queue_empty.json")),
        )
        .mount(&server)
        .await;

    let items = new_sabnzbd_client(&server.uri())
        .list_queue()
        .await
        .expect("empty queue should succeed");
    assert!(items.is_empty());
}

#[tokio::test]
async fn sabnzbd_list_queue_item_has_correct_fields() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "queue"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/queue.json")),
        )
        .mount(&server)
        .await;

    let items = new_sabnzbd_client(&server.uri())
        .list_queue()
        .await
        .unwrap();

    let first = &items[0];
    assert_eq!(first.download_client_item_id, "SABnzbd_nzo_kyt1f0");
    assert_eq!(first.title_name, "My.Movie.2024.1080p.BluRay");
    assert_eq!(first.client_type, "sabnzbd");
    assert_eq!(first.progress_percent, 60);
    assert!(first.size_bytes.is_some());
    assert!(first.remaining_seconds.is_some());

    let second = &items[1];
    assert_eq!(second.download_client_item_id, "SABnzbd_nzo_xyz789");
    assert!(matches!(
        second.state,
        scryer_domain::DownloadQueueState::Queued
    ));
}

// ---------------------------------------------------------------------------
// list_history
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sabnzbd_list_history_filters_old_entries() {
    let server = MockServer::start().await;
    // Use original fixture with old timestamps — should filter out everything
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "history"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/history.json")),
        )
        .mount(&server)
        .await;

    let items = new_sabnzbd_client(&server.uri())
        .list_history()
        .await
        .expect("list_history should succeed even with old entries");
    assert!(
        items.is_empty(),
        "old entries beyond 7-day cutoff should be filtered out"
    );
}

#[tokio::test]
async fn sabnzbd_list_history_recent_entries() {
    let server = MockServer::start().await;
    let now = chrono::Utc::now().timestamp();
    let history = load_fixture("sabnzbd/history.json")
        .replace("1706832000", &now.to_string())
        .replace("1706745600", &(now - 3600).to_string());

    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "history"))
        .respond_with(ResponseTemplate::new(200).set_body_string(history))
        .mount(&server)
        .await;

    let items = new_sabnzbd_client(&server.uri())
        .list_history()
        .await
        .unwrap();
    assert_eq!(items.len(), 2, "recent entries should pass 7-day cutoff");
}

#[tokio::test]
async fn sabnzbd_list_history_maps_statuses() {
    let server = MockServer::start().await;
    let now = chrono::Utc::now().timestamp();
    let history = load_fixture("sabnzbd/history.json")
        .replace("1706832000", &now.to_string())
        .replace("1706745600", &(now - 3600).to_string());

    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "history"))
        .respond_with(ResponseTemplate::new(200).set_body_string(history))
        .mount(&server)
        .await;

    let items = new_sabnzbd_client(&server.uri())
        .list_history()
        .await
        .unwrap();

    let completed = items
        .iter()
        .find(|i| i.title_name.contains("Completed"))
        .unwrap();
    assert!(matches!(
        completed.state,
        scryer_domain::DownloadQueueState::Completed
    ));
    assert_eq!(completed.progress_percent, 100);

    let failed = items
        .iter()
        .find(|i| i.title_name.contains("Failed"))
        .unwrap();
    assert!(matches!(
        failed.state,
        scryer_domain::DownloadQueueState::Failed
    ));
    assert!(failed.attention_required);
}

// ---------------------------------------------------------------------------
// list_completed_downloads
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sabnzbd_list_completed_downloads() {
    let server = MockServer::start().await;
    let now = chrono::Utc::now().timestamp();
    let history = load_fixture("sabnzbd/history.json")
        .replace("1706832000", &now.to_string())
        .replace("1706745600", &(now - 3600).to_string());

    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "history"))
        .respond_with(ResponseTemplate::new(200).set_body_string(history))
        .mount(&server)
        .await;

    let items = new_sabnzbd_client(&server.uri())
        .list_completed_downloads()
        .await
        .expect("list_completed_downloads should succeed");

    assert_eq!(items.len(), 1, "only Completed entries should be returned");
    assert!(items[0].dest_dir.contains("Completed"));
    assert_eq!(items[0].client_type, "sabnzbd");
}

// ---------------------------------------------------------------------------
// pause / resume / delete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sabnzbd_pause_queue_item() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "queue"))
        .and(query_param("name", "pause"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(load_fixture("sabnzbd/pause_resume_success.json")),
        )
        .mount(&server)
        .await;

    let result = new_sabnzbd_client(&server.uri())
        .pause_queue_item("SABnzbd_nzo_kyt1f0")
        .await;
    assert!(result.is_ok(), "pause should succeed: {:?}", result.err());
}

#[tokio::test]
async fn sabnzbd_resume_queue_item() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "queue"))
        .and(query_param("name", "resume"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(load_fixture("sabnzbd/pause_resume_success.json")),
        )
        .mount(&server)
        .await;

    let result = new_sabnzbd_client(&server.uri())
        .resume_queue_item("SABnzbd_nzo_kyt1f0")
        .await;
    assert!(result.is_ok(), "resume should succeed: {:?}", result.err());
}

#[tokio::test]
async fn sabnzbd_delete_queue_item() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "queue"))
        .and(query_param("name", "delete"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/delete_success.json")),
        )
        .mount(&server)
        .await;

    let result = new_sabnzbd_client(&server.uri())
        .delete_queue_item("SABnzbd_nzo_kyt1f0", false)
        .await;
    assert!(result.is_ok(), "delete should succeed: {:?}", result.err());
}

#[tokio::test]
async fn sabnzbd_delete_history_item() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "history"))
        .and(query_param("name", "delete"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/delete_success.json")),
        )
        .mount(&server)
        .await;

    let result = new_sabnzbd_client(&server.uri())
        .delete_queue_item("SABnzbd_nzo_hist01", true)
        .await;
    assert!(
        result.is_ok(),
        "history delete should succeed: {:?}",
        result.err()
    );
}

// ---------------------------------------------------------------------------
// submit_to_download_queue
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sabnzbd_submit_download() {
    // Mock server for both the NZB download and the SABnzbd API
    let server = MockServer::start().await;

    // Mock: NZB file download from indexer
    Mock::given(method("GET"))
        .and(path("/getnzb"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(b"<?xml version=\"1.0\"?><nzb></nzb>".to_vec()),
        )
        .mount(&server)
        .await;

    // Mock: SABnzbd addfile (POST with multipart)
    Mock::given(method("POST"))
        .and(path("/api"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/addurl.json")),
        )
        .mount(&server)
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
        poster_source_url: None,
        banner_url: None,
        banner_source_url: None,
        background_url: None,
        background_source_url: None,
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
        tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    };

    let nzb_url = format!("{}/getnzb?id=abc123&apikey=xyz", server.uri());
    let result = new_submit_sabnzbd_client(&server.uri())
        .await
        .submit_to_download_queue(
            &title,
            Some(nzb_url),
            None,
            None,
            None,
            Some("movies".to_string()),
        )
        .await;

    assert!(result.is_ok(), "submit should succeed: {:?}", result.err());
    let grab = result.unwrap();
    assert_eq!(grab.job_id, "SABnzbd_nzo_abc123");
    assert_eq!(grab.client_type, "sabnzbd");
}

#[tokio::test]
async fn sabnzbd_submit_download_no_source_hint() {
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
        poster_source_url: None,
        banner_url: None,
        banner_source_url: None,
        background_url: None,
        background_source_url: None,
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
        tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    };

    let server = MockServer::start().await;
    let result = new_submit_sabnzbd_client(&server.uri())
        .await
        .submit_to_download_queue(&title, None, None, None, None, None)
        .await;
    assert!(result.is_err(), "should fail without source_hint");
}

#[tokio::test]
async fn sabnzbd_submit_download_deletes_self_staged_nzb_on_failure() {
    let server = MockServer::start().await;
    let staged_nzb_store = new_staged_nzb_store().await;

    Mock::given(method("GET"))
        .and(path("/getnzb"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(load_fixture("nzbgeek/nzb_content.xml").into_bytes()),
        )
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/api"))
        .respond_with(ResponseTemplate::new(500).set_body_string("addfile failed"))
        .mount(&server)
        .await;

    let client = SabnzbdDownloadClient::with_staged_nzb_store(
        server.uri(),
        "test-api-key".to_string(),
        staged_nzb_store.clone(),
        Arc::new(Semaphore::new(4)),
    );

    let error = client
        .submit_to_download_queue(
            &test_title("Broken SAB Submit"),
            Some(format!("{}/getnzb?id=broken", server.uri())),
            Some(DownloadSourceKind::NzbUrl),
            Some("Broken.SAB.Release".to_string()),
            None,
            Some("movies".to_string()),
        )
        .await
        .expect_err("submit should fail");

    assert!(error.to_string().contains("500") || error.to_string().contains("failed"));
    assert_eq!(staged_nzb_store.count_staged_artifacts().await.unwrap(), 0);
}

#[tokio::test]
async fn sabnzbd_submit_download_uses_staged_cache_entry_without_refetch() {
    let server = MockServer::start().await;
    let staged_nzb_store = new_staged_nzb_store().await;
    let nzb_xml = load_fixture("nzbgeek/nzb_content.xml");

    Mock::given(method("POST"))
        .and(path("/api"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/addurl.json")),
        )
        .mount(&server)
        .await;

    let staged = staged_nzb_store
        .stage_nzb_bytes_for_test(nzb_xml.as_bytes())
        .await
        .expect("staged artifact should insert");

    let client = SabnzbdDownloadClient::with_staged_nzb_store(
        server.uri(),
        "test-api-key".to_string(),
        staged_nzb_store.clone(),
        Arc::new(Semaphore::new(4)),
    );

    let result = client
        .submit_download(&request_with_staged_nzb(
            test_title("Staged SAB"),
            staged,
            "Staged.SAB.Release",
        ))
        .await
        .expect("submit should use staged nzb");

    assert_eq!(result.client_type, "sabnzbd");
    assert_eq!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.method.as_str() == "GET")
            .count(),
        0
    );
    assert_eq!(staged_nzb_store.count_staged_artifacts().await.unwrap(), 1);
}

// ===========================================================================
// Weaver integration tests
// ===========================================================================

#[tokio::test]
async fn weaver_submit_download_uses_staged_cache_entry_without_refetch() {
    let server = MockServer::start().await;
    let staged_nzb_store = new_staged_nzb_store().await;
    let nzb_xml = load_fixture("nzbgeek/nzb_content.xml");

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {
                "submitNzb": {
                    "accepted": true,
                    "clientRequestId": "scryer:title-staged-weaver:Staged.Weaver.Release",
                    "item": {
                        "id": 42,
                        "name": "Staged.Weaver.Release",
                        "state": "QUEUED"
                    }
                }
            }
        })))
        .mount(&server)
        .await;

    let staged = staged_nzb_store
        .stage_nzb_bytes_for_test(nzb_xml.as_bytes())
        .await
        .expect("staged artifact should insert");

    let client = WeaverDownloadClient::with_staged_nzb_store(
        server.uri(),
        Some("test-api-key".to_string()),
        staged_nzb_store.clone(),
        Arc::new(Semaphore::new(4)),
    );

    let result = client
        .submit_download(&request_with_staged_nzb(
            test_title("Staged Weaver"),
            staged,
            "Staged.Weaver.Release",
        ))
        .await
        .expect("submit should use staged nzb");

    assert_eq!(result.client_type, "weaver");
    assert_eq!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.method.as_str() == "GET")
            .count(),
        0
    );
    assert_eq!(staged_nzb_store.count_staged_artifacts().await.unwrap(), 1);
}
