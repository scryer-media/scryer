#![recursion_limit = "256"]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::any;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use wiremock::matchers::{body_json_string, body_string_contains, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestContext, load_fixture};
use scryer_application::{
    AppError, DownloadClient, DownloadClientAddRequest, DownloadClientPluginProvider,
    DownloadSourceKind, DownloadSubmissionPurpose, NullSettingsRepository, NullStagedNzbStore,
    StagedNzbRef,
};
use scryer_domain::DownloadClientConfig;
use scryer_infrastructure_acquisition::downloads::{
    clients::{
        NzbgetDownloadClient, PrioritizedDownloadClientRouter, SabnzbdDownloadClient,
        WeaverDownloadClient,
    },
    config_store::DownloadClientConfigStore,
    staged_nzb_store::FileSystemStagedNzbStore,
};
use scryer_plugins::WasmDownloadClientPluginProvider;

fn new_nzbget_client(
    uri: &str,
) -> scryer_infrastructure_acquisition::downloads::clients::NzbgetDownloadClient {
    scryer_infrastructure_acquisition::downloads::clients::NzbgetDownloadClient::new(
        uri.to_string(),
        Some("test-user".to_string()),
        Some("test-pass".to_string()),
        "SCORE".to_string(),
    )
}

async fn new_submit_nzbget_client(
    uri: &str,
) -> scryer_infrastructure_acquisition::downloads::clients::NzbgetDownloadClient {
    scryer_infrastructure_acquisition::downloads::clients::NzbgetDownloadClient::with_staged_nzb_store(
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
        library_id: scryer_domain::default_library_id_for_facet(&scryer_domain::MediaFacet::Movie),
        monitored: true,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: vec![],
        root_folder_id: scryer_domain::root_folder_id_for_path("/data/movies"),
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2024),
        overview: None,
        poster_url: None,
        poster_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        catalog_sort_key: String::new(),
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        popularity: None,
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

#[derive(Clone, Copy)]
enum QbMockMode {
    StatusReauth,
    CompletedDownloads,
}

#[derive(Clone)]
struct QbMockState {
    mode: QbMockMode,
    login_count: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<String>>>,
}

struct QbMockServerHandle {
    base_url: String,
    login_count: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<String>>>,
}

async fn spawn_qbittorrent_mock_server(mode: QbMockMode) -> QbMockServerHandle {
    let state = QbMockState {
        mode,
        login_count: Arc::new(AtomicUsize::new(0)),
        requests: Arc::new(Mutex::new(Vec::new())),
    };
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind qbittorrent mock server");
    let address = listener.local_addr().expect("qbittorrent mock local addr");
    let app = Router::new()
        .fallback(any(qbittorrent_mock_handler))
        .with_state(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("run qbittorrent mock server");
    });

    QbMockServerHandle {
        base_url: format!("http://{address}"),
        login_count: state.login_count,
        requests: state.requests,
    }
}

async fn qbittorrent_mock_handler(
    State(state): State<QbMockState>,
    request: Request<Body>,
) -> impl IntoResponse {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let origin = request
        .headers()
        .get("origin")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let referer = request
        .headers()
        .get("referer")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let cookie = request
        .headers()
        .get("cookie")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = to_bytes(request.into_body(), usize::MAX)
        .await
        .expect("read qbittorrent mock request body");
    let body = String::from_utf8_lossy(&body).to_string();
    state
        .requests
        .lock()
        .expect("qbittorrent mock request log")
        .push(format!(
            "{} {} origin={} referer={} cookie={} body={}",
            method, uri, origin, referer, cookie, body
        ));

    if method.as_str() == "POST"
        && uri.path() == "/api/v2/auth/login"
        && !qbittorrent_mock_browser_headers_ok(&origin, &referer)
    {
        return (StatusCode::FORBIDDEN, "Forbidden").into_response();
    }
    if uri.path().starts_with("/api/v2/")
        && uri.path() != "/api/v2/auth/login"
        && !qbittorrent_mock_api_headers_ok(&origin, &referer, &cookie)
    {
        return (StatusCode::FORBIDDEN, "Forbidden").into_response();
    }

    match (method.as_str(), uri.path()) {
        ("POST", "/api/v2/auth/login") => {
            let login_number = state.login_count.fetch_add(1, Ordering::SeqCst) + 1;
            let cookie_value = match state.mode {
                QbMockMode::StatusReauth if login_number == 1 => "SID=stale".to_string(),
                _ => format!("SID=fresh-{login_number}"),
            };
            (
                StatusCode::OK,
                [
                    ("Content-Type", "text/plain; charset=utf-8"),
                    ("Set-Cookie", &format!("{cookie_value}; HttpOnly")),
                ],
                "Ok.",
            )
                .into_response()
        }
        ("GET", "/api/v2/app/version") => {
            (StatusCode::OK, [("Content-Type", "text/plain")], "4.6.1").into_response()
        }
        ("GET", "/api/v2/app/preferences") => {
            if matches!(state.mode, QbMockMode::StatusReauth) && cookie.contains("SID=stale") {
                return (StatusCode::FORBIDDEN, "Forbidden").into_response();
            }
            (
                StatusCode::OK,
                [("Content-Type", "application/json")],
                r#"{"save_path":"/downloads/base","auto_tmm_enabled":true,"queueing_enabled":false}"#,
            )
                .into_response()
        }
        ("GET", "/api/v2/torrents/categories") => (
            StatusCode::OK,
            [("Content-Type", "application/json")],
            r#"{"series":{"savePath":"/downloads/series"},"movies":{"savePath":"/downloads/movies"}}"#,
        )
            .into_response(),
        ("GET", "/api/v2/torrents/info") => (
            StatusCode::OK,
            [("Content-Type", "application/json")],
            r#"[
  {
    "hash":"AAAABBBBCCCCDDDDEEEEFFFF0000111122223333",
    "name":"Single File Torrent",
    "state":"uploading",
    "category":"movies",
    "save_path":"/downloads/movies",
    "content_path":"/downloads/movies/Single.File.2026.1080p.mkv",
    "size":1234,
    "total_size":1234,
    "amount_left":0,
    "eta":0,
    "progress":1.0,
    "completion_on":1710000000,
    "tags":"scryer-origin,scryer-title-title-1",
    "uploaded":10,
    "downloaded":1234,
    "upspeed":0,
    "dlspeed":0,
    "ratio":1.25,
    "seeding_time":600
  },
  {
    "hash":"BBBBCCCCDDDDEEEEFFFF00001111222233334444",
    "name":"Directory Torrent",
    "state":"pausedup",
    "category":"series",
    "save_path":"/downloads/series",
    "content_path":"/downloads/series/Directory.Torrent.S01",
    "size":4321,
    "total_size":4321,
    "amount_left":0,
    "eta":0,
    "progress":1.0,
    "completion_on":1710000001,
    "tags":"scryer-origin,scryer-facet-series",
    "uploaded":11,
    "downloaded":4321,
    "upspeed":0,
    "dlspeed":0,
    "ratio":1.5,
    "seeding_time":900
  }
]"#,
        )
            .into_response(),
        _ => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

fn qbittorrent_mock_browser_headers_ok(origin: &str, referer: &str) -> bool {
    let origin = origin.trim().trim_end_matches('/');
    let referer = referer.trim().trim_end_matches('/');
    !origin.is_empty()
        && !referer.is_empty()
        && origin == referer
        && (origin.starts_with("http://") || origin.starts_with("https://"))
}

fn qbittorrent_mock_api_headers_ok(origin: &str, referer: &str, cookie: &str) -> bool {
    qbittorrent_mock_browser_headers_ok(origin, referer)
        && cookie
            .split(';')
            .any(|part| part.trim().starts_with("SID="))
}

fn qbittorrent_wasm_bytes() -> Vec<u8> {
    let artifact_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("scryer crate parent")
        .parent()
        .expect("workspace repo parent")
        .parent()
        .expect("workspace container parent")
        .join("scryer-plugins")
        .join("dist")
        .join("qbittorrent_download_client.wasm");
    std::fs::read(&artifact_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", artifact_path.display()))
}

fn qbittorrent_wasm_client(base_url: &str) -> Arc<dyn DownloadClient> {
    let wasm_bytes = qbittorrent_wasm_bytes();
    let provider = WasmDownloadClientPluginProvider::empty().with_external_bytes(&wasm_bytes);
    let provider_types = provider.available_provider_types();
    assert!(
        provider_types
            .iter()
            .any(|provider_type| provider_type == "qbittorrent"),
        "qbittorrent provider should load from dist artifact, saw {provider_types:?}"
    );
    let use_ssl = base_url.trim_start().starts_with("https://");
    let host_and_path = base_url
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    let (host_port, url_base) = host_and_path.split_once('/').unwrap_or((host_and_path, ""));
    let (host, port) = host_port
        .rsplit_once(':')
        .expect("qbittorrent mock host:port");
    let config = DownloadClientConfig {
        id: "qb-wasm".to_string(),
        name: "qBittorrent WASM".to_string(),
        client_type: "qbittorrent".to_string(),
        config_json: json!({
            "host": host,
            "port": port,
            "use_ssl": use_ssl,
            "url_base": url_base,
            "username": "test-user",
            "password": "test-pass",
        })
        .to_string(),
        client_priority: 0,
        is_enabled: true,
        status: scryer_domain::DownloadClientStatus::Healthy,
        last_error: None,
        last_seen_at: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        proxy_config_id: None,
    };
    provider
        .client_for_config(&config)
        .expect("create qbittorrent wasm client")
}

fn request_with_staged_nzb(
    title: scryer_domain::Title,
    staged_nzb: StagedNzbRef,
    source_title: &str,
) -> DownloadClientAddRequest {
    DownloadClientAddRequest {
        search_facet: None,
        title,
        download_id: None,
        source_hint: None,
        staged_nzb: Some(staged_nzb),
        resolved_download_artifact: None,
        source_kind: Some(DownloadSourceKind::NzbFile),
        source_title: Some(source_title.to_string()),
        source_password: None,
        category: Some("movies".to_string()),
        queue_priority: None,
        download_directory: None,
        release_title: None,
        indexer_name: None,
        indexer_id: None,
        info_hash_hint: None,
        seed_goal_ratio: None,
        seed_goal_seconds: None,
        tracker_min_seed_ratio: None,
        tracker_min_seed_time_minutes: None,
        season_pack_seed_ratio: None,
        season_pack_seed_time_minutes: None,
        is_recent: None,
        season_pack: None,
        purpose: DownloadSubmissionPurpose::Standard,
    }
}

fn download_client_config_repo(
    ctx: &TestContext,
) -> Arc<dyn scryer_application::DownloadClientConfigRepository> {
    Arc::new(DownloadClientConfigStore::new(
        ctx.db.datastore(),
        ctx.db.encryption_key_state(),
    ))
}

async fn insert_download_client_config(ctx: &TestContext, config: DownloadClientConfig) {
    download_client_config_repo(ctx)
        .create(config)
        .await
        .expect("create download client config");
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
    let client = scryer_infrastructure_acquisition::downloads::clients::NzbgetDownloadClient::new(
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
    assert_eq!(first.category.as_deref(), Some("movies"));
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
    assert_eq!(items[0].category.as_deref(), Some("movies"));
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
        .delete_queue_item("12345", false, false)
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
        .delete_queue_item("999", true, false)
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
    assert_eq!(items[0].category.as_deref(), Some("movies"));
}

#[tokio::test]
async fn nzbget_list_completed_downloads_includes_non_scryer_entries() {
    let ctx = TestContext::new().await;
    let now = chrono::Utc::now().timestamp();
    let history = load_fixture("nzbget/history.json")
        .replace("1706832000", &now.to_string())
        .replace("1706745600", &(now - 3600).to_string())
        .replace(
            r#"        {"Name": "*scryer_title_id", "Value": "test-title-id"},"#,
            "",
        );

    Mock::given(method("POST"))
        .and(path("/jsonrpc"))
        .respond_with(ResponseTemplate::new(200).set_body_string(history))
        .mount(&ctx.nzbget_server)
        .await;

    let items = new_nzbget_client(&ctx.nzbget_server.uri())
        .list_completed_downloads()
        .await
        .expect("list_completed_downloads should succeed");

    assert_eq!(
        items.len(),
        1,
        "completed NZBGet entries should not require Scryer metadata"
    );
    assert_eq!(items[0].download_client_item_id, "999");
    assert!(
        !items[0]
            .parameters
            .iter()
            .any(|(key, _)| key == "*scryer_title_id")
    );
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

    let title = test_title("Test Movie Title");

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

    let title = test_title("Test Movie Title");

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
    let title = test_title("Test");

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

    assert!(matches!(error, AppError::DownloadSubmitUnavailable(_)));
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
    let client = scryer_infrastructure_acquisition::downloads::clients::NzbgetDownloadClient::new(
        "http://localhost:6789".to_string(),
        None,
        None,
        "SCORE".to_string(),
    );
    assert_eq!(client.endpoint(), "http://localhost:6789/jsonrpc");
}

#[tokio::test]
async fn nzbget_endpoint_preserves_existing_jsonrpc() {
    let client = scryer_infrastructure_acquisition::downloads::clients::NzbgetDownloadClient::new(
        "http://localhost:6789/jsonrpc".to_string(),
        None,
        None,
        "SCORE".to_string(),
    );
    assert_eq!(client.endpoint(), "http://localhost:6789/jsonrpc");
}

#[tokio::test]
async fn nzbget_endpoint_strips_trailing_slash() {
    let client = scryer_infrastructure_acquisition::downloads::clients::NzbgetDownloadClient::new(
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
        proxy_config_id: None,
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

/// Create a router backed by the test DB.
fn build_router(ctx: &TestContext) -> PrioritizedDownloadClientRouter {
    build_router_with_cache(ctx, Arc::new(NullStagedNzbStore))
}

fn build_router_with_cache(
    ctx: &TestContext,
    staged_nzb_store: Arc<dyn scryer_application::StagedNzbStore>,
) -> PrioritizedDownloadClientRouter {
    PrioritizedDownloadClientRouter::new(
        download_client_config_repo(ctx),
        Arc::new(NullSettingsRepository),
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
    insert_download_client_config(&ctx, router_config("c2", &second_server.uri(), 2, true)).await;
    insert_download_client_config(&ctx, router_config("c1", &ctx.nzbget_server.uri(), 1, true))
        .await;

    let router = build_router(&ctx);
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

    insert_download_client_config(&ctx, router_config("c1", &ctx.nzbget_server.uri(), 1, true))
        .await;
    insert_download_client_config(&ctx, router_config("c2", &second_server.uri(), 2, true)).await;

    let router = build_router(&ctx);
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
async fn router_returns_empty_queue_when_no_clients_configured() {
    let ctx = TestContext::new().await;

    mount_list_queue_mocks(&ctx.nzbget_server).await;
    let router = build_router(&ctx);

    let items = router
        .list_queue()
        .await
        .expect("no configured clients should produce an empty queue");

    assert!(items.is_empty());
    assert!(
        ctx.nzbget_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
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
        proxy_config_id: None,
    };
    insert_download_client_config(&ctx, bad_config).await;

    // Priority 2: valid nzbget client, mocked to succeed.
    let second_server = MockServer::start().await;
    mount_list_queue_mocks(&second_server).await;
    insert_download_client_config(&ctx, router_config("good", &second_server.uri(), 2, true)).await;

    let router = build_router(&ctx);
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
        proxy_config_id: None,
    };
    insert_download_client_config(&ctx, no_url_config).await;

    // Priority 2: valid config.
    mount_list_queue_mocks(&ctx.nzbget_server).await;
    insert_download_client_config(
        &ctx,
        router_config("valid", &ctx.nzbget_server.uri(), 2, true),
    )
    .await;

    let router = build_router(&ctx);
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
    insert_download_client_config(
        &ctx,
        router_config("disabled", &ctx.nzbget_server.uri(), 1, false),
    )
    .await;

    let router = build_router(&ctx);

    let items = router
        .list_queue()
        .await
        .expect("only disabled clients should produce an empty queue");

    assert!(items.is_empty());
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

    insert_download_client_config(
        &ctx,
        router_config("primary", &ctx.nzbget_server.uri(), 1, true),
    )
    .await;
    insert_download_client_config(
        &ctx,
        router_config("secondary", &second_client_server.uri(), 2, true),
    )
    .await;

    let router = build_router_with_cache(&ctx, staged_nzb_store.clone());
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

    insert_download_client_config(
        &ctx,
        router_config("primary", &ctx.nzbget_server.uri(), 1, true),
    )
    .await;
    insert_download_client_config(
        &ctx,
        router_config("secondary", &second_client_server.uri(), 2, true),
    )
    .await;

    let router = build_router_with_cache(&ctx, staged_nzb_store.clone());
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

fn new_sabnzbd_credential_client(uri: &str) -> SabnzbdDownloadClient {
    SabnzbdDownloadClient::with_auth(
        uri.to_string(),
        None,
        Some("test-user".to_string()),
        Some("test-pass".to_string()),
    )
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
async fn sabnzbd_test_connection_accepts_username_password_auth() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api"))
        .and(body_string_contains("mode=version"))
        .and(body_string_contains("ma_username=test-user"))
        .and(body_string_contains("ma_password=test-pass"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/version.json")),
        )
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/api"))
        .and(body_string_contains("mode=queue"))
        .and(body_string_contains("ma_username=test-user"))
        .and(body_string_contains("ma_password=test-pass"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/queue_empty.json")),
        )
        .mount(&server)
        .await;

    let result = new_sabnzbd_credential_client(&server.uri())
        .test_connection()
        .await;
    assert_eq!(result.unwrap(), "4.5.1");

    let requests = server.received_requests().await.unwrap();
    let version_request = requests
        .iter()
        .find(|request| {
            request.method.as_str() == "POST"
                && request.url.path() == "/api"
                && String::from_utf8_lossy(&request.body).contains("mode=version")
        })
        .expect("credential version auth request should be posted");
    let body = String::from_utf8_lossy(&version_request.body);
    assert!(body.contains("mode=version"));
    assert!(body.contains("ma_username=test-user"));
    assert!(body.contains("ma_password=test-pass"));

    let queue_request = requests
        .iter()
        .find(|request| {
            request.method.as_str() == "POST"
                && request.url.path() == "/api"
                && String::from_utf8_lossy(&request.body).contains("mode=queue")
        })
        .expect("credential queue auth request should be posted");
    let body = String::from_utf8_lossy(&queue_request.body);
    assert!(body.contains("mode=queue"));
    assert!(body.contains("ma_username=test-user"));
    assert!(body.contains("ma_password=test-pass"));
}

#[tokio::test]
async fn sabnzbd_test_connection_supports_base_url_prefix() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/sabnzbd/api"))
        .and(query_param("mode", "version"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/version.json")),
        )
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/sabnzbd/api"))
        .and(query_param("mode", "queue"))
        .and(query_param("apikey", "test-api-key"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/queue_empty.json")),
        )
        .mount(&server)
        .await;

    let client = SabnzbdDownloadClient::new(
        format!("{}/sabnzbd/", server.uri()),
        "test-api-key".to_string(),
    );
    let result = client.test_connection().await;
    assert_eq!(result.unwrap(), "4.5.1");
}

#[tokio::test]
async fn sabnzbd_test_connection_falls_back_to_authenticated_version_when_required() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "version"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "status": false,
            "error": "API Key Required"
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/api"))
        .and(body_string_contains("mode=version"))
        .and(body_string_contains("ma_username=test-user"))
        .and(body_string_contains("ma_password=test-pass"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/version.json")),
        )
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/api"))
        .and(body_string_contains("mode=queue"))
        .and(body_string_contains("ma_username=test-user"))
        .and(body_string_contains("ma_password=test-pass"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/queue_empty.json")),
        )
        .mount(&server)
        .await;

    let result = new_sabnzbd_credential_client(&server.uri())
        .test_connection()
        .await;
    assert_eq!(result.unwrap(), "4.5.1");
}

#[tokio::test]
async fn sabnzbd_prefers_api_key_over_username_password_when_both_are_present() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "version"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/version.json")),
        )
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "queue"))
        .and(query_param("apikey", "preferred-key"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/queue_empty.json")),
        )
        .mount(&server)
        .await;

    let client = SabnzbdDownloadClient::with_auth(
        server.uri(),
        Some("preferred-key".to_string()),
        Some("test-user".to_string()),
        Some("test-pass".to_string()),
    );
    let result = client.test_connection().await;
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
        result
            .unwrap_err()
            .to_string()
            .contains("authentication validation failed"),
        "error should mention authentication validation"
    );
}

#[tokio::test]
async fn sabnzbd_get_client_status_reports_output_roots_and_sorting_mode() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "get_config"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/config.json")),
        )
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "fullstatus"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/fullstatus.json")),
        )
        .mount(&server)
        .await;

    let status = new_sabnzbd_client(&server.uri())
        .get_client_status()
        .await
        .expect("status should succeed");

    assert_eq!(status.is_localhost, Some(true));
    assert_eq!(status.sorting_mode.as_deref(), Some("TV"));
    assert_eq!(status.removes_completed_downloads, Some(true));
    assert_eq!(
        status.remote_output_roots,
        vec![
            "/srv/downloads/complete".to_string(),
            "/srv/downloads/complete/series".to_string(),
            "/srv/downloads/complete/movies".to_string(),
            "/srv/downloads/complete/anime".to_string(),
        ]
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
async fn sabnzbd_list_queue_accepts_top_level_slots_variant() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "queue"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": true,
            "slots": []
        })))
        .mount(&server)
        .await;

    let items = new_sabnzbd_client(&server.uri())
        .list_queue()
        .await
        .expect("top-level queue slots should be accepted");
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
    assert_eq!(first.category.as_deref(), Some("movies"));
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
    assert_eq!(items[0].category.as_deref(), Some("movies"));
}

#[tokio::test]
async fn sabnzbd_list_history_accepts_top_level_slots_variant() {
    let server = MockServer::start().await;
    let now = chrono::Utc::now().timestamp();

    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "history"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": true,
            "slots": [{
                "nzo_id": "hist-top-1",
                "name": "Top Level History",
                "status": "Completed",
                "completed": now,
                "bytes": 1234,
                "time_added": now - 60
            }]
        })))
        .mount(&server)
        .await;

    let items = new_sabnzbd_client(&server.uri())
        .list_history()
        .await
        .expect("top-level history slots should be accepted");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].download_client_item_id, "hist-top-1");
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
    assert_eq!(items[0].category.as_deref(), Some("movies"));
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
        .delete_queue_item("SABnzbd_nzo_kyt1f0", false, false)
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
        .delete_queue_item("SABnzbd_nzo_hist01", true, false)
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

    let title = test_title("Test Movie Title");

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
    let title = test_title("Test");

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

    // Resolve the API path to `/api`, and keep the queue/history empty so the
    // ambiguous-addfile reconciliation finds nothing and surfaces the
    // ambiguous error.
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "queue"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/queue_empty.json")),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "history"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"history":{"slots":[]}}"#))
        .mount(&server)
        .await;
    // The addfile POST fails after the upload was sent → ambiguous, not a
    // definitive rejection. With path-pinning there is no re-POST to the
    // alternate `/sabnzbd/api` path.
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

    assert!(
        matches!(error, AppError::DownloadSubmitAmbiguous(_)),
        "expected ambiguous submit error, got {error:?}"
    );
    // The addfile POST must not have been re-sent to the alternate path.
    let addfile_posts = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| request.method.as_str() == "POST" && request.url.path() == "/api")
        .count();
    assert_eq!(addfile_posts, 1, "exactly one addfile POST should be sent");
    assert_eq!(staged_nzb_store.count_staged_artifacts().await.unwrap(), 0);
}

#[tokio::test]
async fn sabnzbd_submit_download_reconciles_ambiguous_addfile_from_queue() {
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

    // The probe and the ambiguous-addfile reconciliation both read the queue,
    // which already contains the job we just uploaded (queue.json has a slot
    // named "My.Movie.2024.1080p.BluRay" -> nzo_id "SABnzbd_nzo_kyt1f0").
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "queue"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/queue.json")),
        )
        .mount(&server)
        .await;
    // addfile fails after the upload was sent → ambiguous. Reconciliation must
    // adopt the queued job instead of re-POSTing (which would duplicate it).
    Mock::given(method("POST"))
        .and(path("/api"))
        .respond_with(ResponseTemplate::new(500).set_body_string("gateway boom"))
        .mount(&server)
        .await;

    let client = SabnzbdDownloadClient::with_staged_nzb_store(
        server.uri(),
        "test-api-key".to_string(),
        staged_nzb_store,
        Arc::new(Semaphore::new(4)),
    );

    let result = client
        .submit_to_download_queue(
            &test_title("My.Movie.2024.1080p.BluRay"),
            Some(format!("{}/getnzb?id=movie", server.uri())),
            Some(DownloadSourceKind::NzbUrl),
            Some("My.Movie.2024.1080p.BluRay".to_string()),
            None,
            Some("movies".to_string()),
        )
        .await
        .expect("ambiguous addfile should reconcile to the queued job");

    assert_eq!(result.job_id, "SABnzbd_nzo_kyt1f0");
    assert_eq!(result.client_type, "sabnzbd");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method.as_str() == "POST" && request.url.path() == "/api")
            .count(),
        1,
        "the addfile POST must be sent exactly once"
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.path() == "/sabnzbd/api")
            .count(),
        0,
        "an ambiguous addfile must never re-POST to the alternate path"
    );
}

#[tokio::test]
async fn sabnzbd_submit_download_reconciles_title_with_sab_illegal_characters() {
    // A release title with SAB-illegal characters (`:`) is stored by SAB under
    // a sanitized final_name ("Mission_ Impossible"). Reconciliation must match
    // it client-side and must NOT rely on the server-side `search` param (which
    // matches the sanitized name, not the raw title) — otherwise the landed job
    // would be missed and the next cycle would re-submit into a duplicate.
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
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "queue"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"queue":{"slots":[{"status":"Downloading","filename":"Mission_ Impossible","nzo_id":"SABnzbd_nzo_mi","cat":"movies"}]}}"#,
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api"))
        .respond_with(ResponseTemplate::new(502).set_body_string("bad gateway"))
        .mount(&server)
        .await;

    let client = SabnzbdDownloadClient::with_staged_nzb_store(
        server.uri(),
        "test-api-key".to_string(),
        staged_nzb_store,
        Arc::new(Semaphore::new(4)),
    );

    let result = client
        .submit_to_download_queue(
            &test_title("Mission: Impossible"),
            Some(format!("{}/getnzb?id=mi", server.uri())),
            Some(DownloadSourceKind::NzbUrl),
            Some("Mission: Impossible".to_string()),
            None,
            Some("movies".to_string()),
        )
        .await
        .expect("illegal-char title should still reconcile from the queue");

    assert_eq!(result.job_id, "SABnzbd_nzo_mi");

    // The reconciliation must fetch the queue unfiltered — no `search` param —
    // so a sanitized final_name is still discoverable.
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .filter(|request| request.method.as_str() == "GET" && request.url.path() == "/api")
            .all(|request| request.url.query_pairs().all(|(key, _)| key != "search")),
        "reconciliation must not depend on SAB's server-side search of the sanitized name"
    );
}

#[tokio::test]
async fn sabnzbd_submit_download_resolves_sabnzbd_compat_path() {
    // altmount-style backend: `/api` is a different application; the SAB-compat
    // API is served only under `/sabnzbd/api`. The idempotent probe must
    // discover this and pin the addfile POST to `/sabnzbd/api`.
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
    Mock::given(method("GET"))
        .and(path("/api"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/sabnzbd/api"))
        .and(query_param("mode", "queue"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/queue_empty.json")),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/sabnzbd/api"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/addurl.json")),
        )
        .mount(&server)
        .await;

    let client = SabnzbdDownloadClient::with_staged_nzb_store(
        server.uri(),
        "test-api-key".to_string(),
        staged_nzb_store,
        Arc::new(Semaphore::new(4)),
    );

    let result = client
        .submit_to_download_queue(
            &test_title("Compat Path"),
            Some(format!("{}/getnzb?id=compat", server.uri())),
            Some(DownloadSourceKind::NzbUrl),
            Some("Compat.Path.Release".to_string()),
            None,
            Some("movies".to_string()),
        )
        .await
        .expect("addfile should route to the resolved sabnzbd-compat path");

    assert_eq!(result.job_id, "SABnzbd_nzo_abc123");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(
                |request| request.method.as_str() == "POST" && request.url.path() == "/sabnzbd/api"
            )
            .count(),
        1,
        "addfile should be POSTed to the resolved /sabnzbd/api path"
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method.as_str() == "POST" && request.url.path() == "/api")
            .count(),
        0,
        "addfile must never be POSTed to the unrouted /api path"
    );
}

#[tokio::test]
async fn sabnzbd_submit_download_rejects_definitive_status_false() {
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
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "queue"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/queue_empty.json")),
        )
        .mount(&server)
        .await;
    // A definitive SAB rejection (e.g. duplicate) — never retried, never
    // failed over, and blocklist-worthy downstream.
    Mock::given(method("POST"))
        .and(path("/api"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"status": false, "error": "Duplicate NZB"}"#),
        )
        .mount(&server)
        .await;

    let client = SabnzbdDownloadClient::with_staged_nzb_store(
        server.uri(),
        "test-api-key".to_string(),
        staged_nzb_store,
        Arc::new(Semaphore::new(4)),
    );

    let error = client
        .submit_to_download_queue(
            &test_title("Duplicate Release"),
            Some(format!("{}/getnzb?id=dup", server.uri())),
            Some(DownloadSourceKind::NzbUrl),
            Some("Duplicate.Release".to_string()),
            None,
            Some("movies".to_string()),
        )
        .await
        .expect_err("a definitive rejection should fail the submit");

    assert!(
        matches!(error, AppError::DownloadSubmitRejected(_)),
        "expected a rejected submit error, got {error:?}"
    );
    assert!(
        error.to_string().contains("Duplicate NZB"),
        "rejection should carry SAB's detail: {error}"
    );
}

#[tokio::test]
async fn sabnzbd_submit_download_uses_staged_cache_entry_without_refetch() {
    let server = MockServer::start().await;
    let staged_nzb_store = new_staged_nzb_store().await;
    let nzb_xml = load_fixture("nzbgeek/nzb_content.xml");

    // Idempotent probe used to resolve the SAB API path before the addfile
    // POST (see resolve_addfile_url); pin it to `/api`.
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("mode", "queue"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/queue_empty.json")),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api"))
        .and(query_param("apikey", "test-api-key"))
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
    let requests = server.received_requests().await.unwrap();
    let addfile = requests
        .iter()
        .find(|request| {
            request.method.as_str() == "POST"
                && request
                    .url
                    .query_pairs()
                    .any(|(key, value)| key == "mode" && value == "addfile")
        })
        .expect("an addfile POST should have been sent");
    let request_body = String::from_utf8_lossy(&addfile.body);
    let query_pairs = addfile.url.query_pairs().collect::<Vec<_>>();
    for (key, expected) in [
        ("mode", "addfile"),
        ("output", "json"),
        ("nzbname", "Staged.SAB.Release"),
        ("priority", "-1"),
        ("cat", "movies"),
        ("apikey", "test-api-key"),
    ] {
        assert_eq!(
            query_pairs
                .iter()
                .filter(|(candidate, value)| candidate == key && value == expected)
                .count(),
            1,
            "sabnzbd upload should send query param {key}={expected} exactly once: {:?}",
            addfile.url.query()
        );
    }
    assert!(
        query_pairs
            .iter()
            .any(|(key, value)| key == "apikey" && value == "test-api-key"),
        "sabnzbd upload should authenticate with the API key in the query string: {:?}",
        addfile.url.query()
    );
    assert!(
        request_body.contains("filename=\"Staged.SAB.Release.nzb\""),
        "sabnzbd upload should use the plain nzb filename path: {request_body}"
    );
    assert!(
        request_body.contains("application/x-nzb"),
        "sabnzbd upload should remain a plain nzb upload: {request_body}"
    );
    assert!(
        !request_body.contains("name=\"apikey\""),
        "sabnzbd upload should not duplicate API-key auth in the multipart body: {request_body}"
    );
    for forbidden_field in ["mode", "output", "nzbname", "priority", "cat"] {
        assert!(
            !request_body.contains(&format!("name=\"{forbidden_field}\"")),
            "sabnzbd upload should not duplicate {forbidden_field} in the multipart body: {request_body}"
        );
    }
    // The staged entry must be reused without re-fetching the NZB from the
    // indexer (path-resolution probes are permitted, an NZB refetch is not).
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.path().contains("getnzb"))
            .count(),
        0
    );
    assert_eq!(staged_nzb_store.count_staged_artifacts().await.unwrap(), 1);
}

#[tokio::test]
async fn sabnzbd_submit_download_invalid_api_key_maps_to_authentication_failure() {
    let server = MockServer::start().await;
    let staged_nzb_store = new_staged_nzb_store().await;
    let nzb_xml = load_fixture("nzbgeek/nzb_content.xml");

    Mock::given(method("POST"))
        .and(path("/api"))
        .and(query_param("apikey", "test-api-key"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(load_fixture("sabnzbd/error.json")),
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
        staged_nzb_store,
        Arc::new(Semaphore::new(4)),
    );

    let error = client
        .submit_download(&request_with_staged_nzb(
            test_title("Broken SAB Auth"),
            staged,
            "Broken.SAB.Auth",
        ))
        .await
        .expect_err("submit should fail with auth error");

    assert!(
        error.to_string().contains("authentication failed"),
        "error should mention authentication failure: {error}"
    );
}

#[tokio::test]
async fn sabnzbd_submit_download_accepts_username_password_auth() {
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

    let client = SabnzbdDownloadClient::with_auth_and_staged_nzb_store(
        server.uri(),
        None,
        Some("test-user".to_string()),
        Some("test-pass".to_string()),
        staged_nzb_store,
        Arc::new(Semaphore::new(4)),
    );

    let result = client
        .submit_download(&request_with_staged_nzb(
            test_title("Credential SAB Submit"),
            staged,
            "Credential.SAB.Release",
        ))
        .await;

    assert!(result.is_ok(), "submit should accept credential auth");

    let requests = server.received_requests().await.unwrap();
    let addfile = requests
        .iter()
        .find(|request| {
            request.method.as_str() == "POST"
                && request
                    .url
                    .query_pairs()
                    .any(|(key, value)| key == "mode" && value == "addfile")
        })
        .expect("an addfile POST should have been sent");
    let request_body = String::from_utf8_lossy(&addfile.body);
    let query_pairs = addfile.url.query_pairs().collect::<Vec<_>>();
    for (key, expected) in [
        ("mode", "addfile"),
        ("output", "json"),
        ("nzbname", "Credential.SAB.Release"),
        ("priority", "-1"),
        ("cat", "movies"),
        ("ma_username", "test-user"),
        ("ma_password", "test-pass"),
    ] {
        assert_eq!(
            query_pairs
                .iter()
                .filter(|(candidate, value)| candidate == key && value == expected)
                .count(),
            1,
            "credential-auth SAB upload should send query param {key}={expected} exactly once: {:?}",
            addfile.url.query()
        );
    }
    assert!(
        query_pairs
            .iter()
            .any(|(key, value)| key == "ma_username" && value == "test-user"),
        "credential-auth SAB upload should include ma_username in the query string: {:?}",
        addfile.url.query()
    );
    assert!(
        query_pairs
            .iter()
            .any(|(key, value)| key == "ma_password" && value == "test-pass"),
        "credential-auth SAB upload should include ma_password in the query string: {:?}",
        addfile.url.query()
    );
    assert!(
        query_pairs.iter().all(|(key, _)| key != "apikey"),
        "credential-auth SAB upload should not send an API key: {:?}",
        addfile.url.query()
    );
    assert!(
        !request_body.contains("name=\"apikey\""),
        "credential-auth SAB upload should not include API-key fields in the multipart body: {request_body}"
    );
    for forbidden_field in [
        "mode",
        "output",
        "nzbname",
        "priority",
        "cat",
        "ma_username",
        "ma_password",
    ] {
        assert!(
            !request_body.contains(&format!("name=\"{forbidden_field}\"")),
            "credential-auth SAB upload should not duplicate {forbidden_field} in the multipart body: {request_body}"
        );
    }
}

// ===========================================================================
// qBittorrent WASM runtime tests
// ===========================================================================

#[tokio::test]
#[ignore = "qBittorrent dist artifact is still on an older plugin SDK line than this host"]
async fn qbittorrent_wasm_status_reauths_after_403_and_reports_output_roots() {
    let server = spawn_qbittorrent_mock_server(QbMockMode::StatusReauth).await;
    let client = qbittorrent_wasm_client(&server.base_url);

    let status = client
        .get_client_status()
        .await
        .expect("qBittorrent status should succeed");

    assert_eq!(server.login_count.load(Ordering::SeqCst), 2);
    assert_eq!(status.version.as_deref(), Some("4.6.1"));
    assert_eq!(status.is_localhost, Some(true));
    assert_eq!(status.sorting_mode.as_deref(), Some("auto_tmm"));
    assert_eq!(
        status.remote_output_roots,
        vec![
            "/downloads/base".to_string(),
            "/downloads/movies".to_string(),
            "/downloads/series".to_string(),
        ]
    );

    let requests = server
        .requests
        .lock()
        .expect("qbittorrent request log")
        .clone();
    assert!(
        requests
            .iter()
            .any(|request| request.contains("/api/v2/auth/login")
                && request.contains("username=test-user")
                && request.contains("password=test-pass")),
        "login request should include username/password: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|request| request.contains("/api/v2/app/preferences")
                && request.contains("SID=stale")),
        "status path should hit preferences with the stale cookie before re-auth: {requests:?}"
    );
}

#[tokio::test]
#[ignore = "qBittorrent dist artifact is still on an older plugin SDK line than this host"]
async fn qbittorrent_wasm_completed_downloads_derive_single_file_and_directory_roots() {
    let server = spawn_qbittorrent_mock_server(QbMockMode::CompletedDownloads).await;
    let client = qbittorrent_wasm_client(&server.base_url);

    let items = client
        .list_completed_downloads()
        .await
        .expect("completed downloads should succeed");

    assert_eq!(server.login_count.load(Ordering::SeqCst), 1);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].name, "Single File Torrent");
    assert_eq!(items[0].dest_dir, "/downloads/movies");
    assert_eq!(items[1].name, "Directory Torrent");
    assert_eq!(items[1].dest_dir, "/downloads/series/Directory.Torrent.S01");
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

#[tokio::test]
async fn weaver_submit_download_unaccepted_response_maps_to_submit_unavailable() {
    let server = MockServer::start().await;
    let staged_nzb_store = new_staged_nzb_store().await;
    let nzb_xml = load_fixture("nzbgeek/nzb_content.xml");

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {
                "submitNzb": {
                    "accepted": false,
                    "clientRequestId": "scryer:title-staged-weaver:Rejected.Weaver.Release",
                    "item": {
                        "id": 42,
                        "name": "Rejected.Weaver.Release",
                        "state": "FAILED"
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

    let error = client
        .submit_download(&request_with_staged_nzb(
            test_title("Rejected Weaver"),
            staged,
            "Rejected.Weaver.Release",
        ))
        .await
        .expect_err("submit should fail");

    assert!(matches!(error, AppError::DownloadSubmitUnavailable(_)));
    assert_eq!(staged_nzb_store.count_staged_artifacts().await.unwrap(), 1);
}
