use super::*;
use crate::diagnostics_routes::DiagnosticsProcessFacts;
use crate::error_reasons::{
    HTTP_DIAGNOSTICS_PERMISSION_DENIED, HTTP_MCP_ARGUMENTS_INVALID, HTTP_MCP_GRAPHQL_ERROR,
    HTTP_MCP_LOG_PATTERN_INVALID, HTTP_MCP_TOOL_UNKNOWN,
};
use crate::log_buffer::LogRingBuffer;
use crate::middleware::integration_test_common as common;
use crate::middleware::{AuthlessWebClientProofState, WebSocketOriginPolicy};
use crate::rate_limit::ScryerRateLimiter;

use axum::Router;
use axum::http::HeaderValue;
use axum::routing::post;
use scryer_application::application_upgrade::InstallationAssessment;
use scryer_application::{
    AcquisitionScopeState, AcquisitionScopeStateRepository, ReleaseDecision, TitleRepository,
};
use scryer_domain::{AppPermissionMask, Id, MediaFacet, Title};
use scryer_interface::context::{AuthRuntimeStateHandle, AuthRuntimeStateSnapshot};
use serde_json::{Value, json};
use std::future::Future;
use std::io::Write as _;
use std::net::Ipv4Addr;
use std::path::Path;
use std::time::Duration;
use tower::ServiceExt as _;

const HOST: &str = "scryer.lan:8080";
/// A failure bound for every await; a correct run never gets near it.
const BOUND: Duration = Duration::from_secs(120);
/// About 5,000 tokens of JSON text.
const RESULT_TEXT_MAX_CHARS: usize = 20_000;

/// Awaits `future`, failing the test instead of hanging it.
async fn within<F: Future>(what: &str, future: F) -> F::Output {
    tokio::time::timeout(BOUND, future)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

struct Fixture {
    context: common::TestContext,
    router: Router,
    admin: scryer_domain::User,
    api_key: String,
    session_token: String,
    logs: LogRingBuffer,
    data_dir: tempfile::TempDir,
}

#[derive(Default)]
struct Options {
    auth_runtime: Option<AuthRuntimeStateHandle>,
    rate_limiter: Option<ScryerRateLimiter>,
    log_lines: Option<usize>,
}

fn auth_state(
    context: &common::TestContext,
    auth_runtime: AuthRuntimeStateHandle,
    rate_limiter: ScryerRateLimiter,
) -> AuthState {
    AuthState {
        app: context.app.clone(),
        schema: context.schema.clone(),
        auth_runtime,
        rate_limiter,
        ws_origin_policy: WebSocketOriginPolicy::default(),
        list_account_origin_policy: Default::default(),
        authless_web_client_proof: AuthlessWebClientProofState::new(),
    }
}

fn form_login_runtime() -> AuthRuntimeStateHandle {
    AuthRuntimeStateHandle::new(AuthRuntimeStateSnapshot {
        form_login_enabled: true,
        skip_login_for_local_ips: false,
        effective_form_login_enabled: true,
        webauthn_configured: false,
        passkey_enabled: false,
        env_override_active: false,
        env_override_description: None,
        epoch: 1,
    })
}

fn diagnostics_host(data_dir: &Path, logs: LogRingBuffer) -> DiagnosticsHost {
    DiagnosticsHost {
        process: DiagnosticsProcessFacts {
            started_at: chrono::Utc::now(),
            log_file: None,
        },
        log_ring_buffer: logs,
        metrics: None,
        installation: InstallationAssessment::default(),
        data_dir: data_dir.to_path_buf(),
        listening: SocketAddr::from((Ipv4Addr::LOCALHOST, 8080)),
        base_path_configured: false,
        // The database directory is registered as a path the sanitizer
        // replaces.
        sqlite_path: Some(data_dir.join("scryer.db")),
    }
}

async fn create_key(context: &common::TestContext, owner: &scryer_domain::User) -> String {
    within(
        "API key",
        context.app.create_api_key(
            owner,
            scryer_application::CreateApiKey {
                label: "mcp-fixture-key".into(),
                expiry: scryer_application::ApiKeyExpiryPreset::Never,
            },
        ),
    )
    .await
    .expect("create API key")
    .raw_key
}

async fn fixture_with(options: Options) -> Fixture {
    let context = within("test context", common::TestContext::new()).await;
    let admin = within(
        "default administrator",
        context.app.find_or_create_default_user(),
    )
    .await
    .expect("default administrator");
    let api_key = create_key(&context, &admin).await;
    let session_token = within("session token", context.app.issue_access_token(&admin))
        .await
        .expect("issue session token");
    let auth_runtime = options
        .auth_runtime
        .unwrap_or_else(|| context.auth_runtime.clone());
    let rate_limiter = options
        .rate_limiter
        .unwrap_or_else(|| ScryerRateLimiter::from_env(Default::default()));
    let data_dir = tempfile::tempdir().expect("diagnostics data dir");
    let logs = LogRingBuffer::new(options.log_lines.unwrap_or(400));
    let host = Arc::new(diagnostics_host(data_dir.path(), logs.clone()));
    let state = McpState::new(auth_state(&context, auth_runtime, rate_limiter), host);
    let router = Router::new()
        .route(MCP_PATH, post(mcp_handler))
        .with_state(state);
    Fixture {
        context,
        router,
        admin,
        api_key,
        session_token,
        logs,
        data_dir,
    }
}

async fn fixture() -> Fixture {
    fixture_with(Options {
        auth_runtime: Some(form_login_runtime()),
        ..Options::default()
    })
    .await
}

fn rpc_request_at(
    version: &str,
    bearer: Option<&str>,
    origin: Option<&str>,
    body: Value,
) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri(MCP_PATH)
        .header(header::HOST, HOST)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header("mcp-protocol-version", version)
        .body(Body::from(body.to_string()))
        .expect("MCP request");
    if let Some(bearer) = bearer {
        request.headers_mut().insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {bearer}")).expect("authorization"),
        );
    }
    if let Some(origin) = origin {
        request.headers_mut().insert(
            header::ORIGIN,
            HeaderValue::from_str(origin).expect("origin"),
        );
    }
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from((Ipv4Addr::LOCALHOST, 3000))));
    request
}

fn rpc_request(bearer: Option<&str>, origin: Option<&str>, body: Value) -> Request<Body> {
    rpc_request_at("2025-11-25", bearer, origin, body)
}

async fn send(
    router: &Router,
    request: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let response = within("MCP response", router.clone().oneshot(request))
        .await
        .expect("MCP response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = within(
        "MCP body",
        axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024),
    )
    .await
    .expect("MCP body");
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            panic!(
                "MCP body is JSON ({error}): {}",
                String::from_utf8_lossy(&bytes)
            )
        })
    };
    (status, headers, body)
}

fn initialize() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "mcp-fixture-client", "version": "0.0.1"}
        }
    })
}

fn list_tools() -> Value {
    json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
}

fn call_tool(name: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments}
    })
}

/// Calls a tool with `key` and returns the JSON-RPC body.
async fn call(fixture: &Fixture, key: &str, name: &str, arguments: Value) -> Value {
    let (status, _, body) = send(
        &fixture.router,
        rpc_request(Some(key), None, call_tool(name, arguments)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{name}: {body}");
    body
}

/// A successful tool result's structured content, checked against the size
/// budget and the text block.
fn success(name: &str, body: &Value) -> Value {
    let result = &body["result"];
    assert_eq!(result["isError"], false, "{name}: {body}");
    let content = result["structuredContent"].clone();
    assert!(content.is_object(), "{name}: {body}");
    let text = result["content"][0]["text"].as_str().expect("text block");
    assert!(
        text.chars().count() <= RESULT_TEXT_MAX_CHARS,
        "{name} returned {} characters",
        text.chars().count()
    );
    assert_eq!(
        serde_json::from_str::<Value>(text).expect("text block is JSON"),
        content,
        "{name}"
    );
    content
}

/// A tool error's first error entry.
fn tool_error(name: &str, body: &Value) -> Value {
    let result = &body["result"];
    assert_eq!(result["isError"], true, "{name}: {body}");
    let error = result["structuredContent"]["errors"][0].clone();
    assert!(error["code"].is_string(), "{name}: {body}");
    assert!(error["message"].is_string(), "{name}: {body}");
    error
}

/// A JSON-RPC protocol error's reason.
fn protocol_error_reason(body: &Value) -> Value {
    assert!(body["error"]["code"].is_i64(), "{body}");
    assert!(body.get("result").is_none(), "{body}");
    body["error"]["data"]["reason"].clone()
}

fn fixture_title(name: &str) -> Title {
    Title {
        id: Id::new().0,
        name: name.to_string(),
        facet: MediaFacet::Movie,
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
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
        movie_release_dates: None,
        folder_path: None,
    }
}

/// Seeds a movie with a wanted item and `codes.len()` decisions, newest
/// last, and returns the title id.
async fn seed_title_with_decisions(context: &common::TestContext, codes: &[&str]) -> String {
    let title = within(
        "create title",
        context.titles.create(fixture_title("Fixture Probe Film")),
    )
    .await
    .expect("create title");
    let wanted_item = AcquisitionScopeState {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: Some(title.facet.as_str().to_string()),
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        last_search_at: None,
        status: scryer_application::AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        landed_bar: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: "2026-03-20T00:00:00Z".to_string(),
        updated_at: "2026-03-20T00:00:00Z".to_string(),
    };
    within(
        "seed wanted item",
        context
            .library_state
            .upsert_acquisition_scope_state(&wanted_item),
    )
    .await
    .expect("seed wanted item");
    for (index, code) in codes.iter().enumerate() {
        let decision = ReleaseDecision {
            id: Id::new().0,
            wanted_item_id: wanted_item.id.clone(),
            title_id: title.id.clone(),
            release_title: format!("Fixture.Probe.Film.2024.1080p.WEB-DL-GRP{index}"),
            release_url: None,
            release_size_bytes: Some(4_096),
            decision_code: (*code).to_string(),
            candidate_score: 90,
            current_score: None,
            score_delta: None,
            explanation_json: None,
            created_at: format!("2026-03-20T00:0{index}:00Z"),
        };
        within(
            "seed decision",
            context.library_state.insert_release_decision(&decision),
        )
        .await
        .expect("seed decision");
    }
    title.id
}

const TOOL_NAMES: &[&str] = &[
    "whoami",
    "get_health_checks",
    "list_download_clients",
    "list_indexers",
    "list_indexer_errors",
    "get_system_health",
    "list_job_runs",
    "find_titles",
    "get_title_decisions",
    "list_pending_releases",
    "get_title_history",
    "list_download_queue",
    "list_import_history",
    "get_upgrade_status",
    "get_diagnostics_overview",
    "read_diagnostics_section",
    "search_service_logs",
];

const DIAGNOSTICS_TOOLS: &[&str] = &[
    "get_diagnostics_overview",
    "read_diagnostics_section",
    "search_service_logs",
];

/// One valid call per tool.
fn minimal_calls(title_id: &str) -> Vec<(&'static str, Value)> {
    vec![
        ("whoami", json!({})),
        ("get_health_checks", json!({})),
        ("list_download_clients", json!({})),
        ("list_indexers", json!({"include_stats": true})),
        ("list_indexer_errors", json!({})),
        ("get_system_health", json!({})),
        ("list_job_runs", json!({})),
        ("find_titles", json!({"query": "Fixture Probe"})),
        ("get_title_decisions", json!({"title_id": title_id})),
        ("list_pending_releases", json!({"title_id": title_id})),
        ("get_title_history", json!({"title_id": title_id})),
        ("list_download_queue", json!({})),
        ("list_import_history", json!({})),
        ("get_upgrade_status", json!({})),
        ("get_diagnostics_overview", json!({})),
        ("read_diagnostics_section", json!({"section": "build"})),
        ("search_service_logs", json!({})),
    ]
}

#[tokio::test]
async fn mcp_refuses_every_caller_that_is_not_an_api_key() {
    // The shared fixture leaves authless local access on, so a request with no
    // credentials would resolve the default administrator on `/graphql`.
    let fixture = fixture_with(Options::default()).await;

    let (status, headers, body) =
        send(&fixture.router, rpc_request(None, None, list_tools())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["reason"], HTTP_MCP_API_KEY_REQUIRED);
    assert_eq!(body["code"], "UNAUTHORIZED");
    assert_eq!(headers[header::WWW_AUTHENTICATE], "Bearer");

    let session = fixture.session_token.clone();
    let (status, headers, body) = send(
        &fixture.router,
        rpc_request(Some(&session), None, list_tools()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["reason"], HTTP_MCP_API_KEY_REQUIRED);
    assert_eq!(
        headers[header::WWW_AUTHENTICATE],
        "Bearer error=\"invalid_token\""
    );

    let oauth_token = within(
        "OAuth token",
        fixture.context.app.issue_oauth_access_token(
            &fixture.admin,
            "generic-native",
            "mcp-fixture-oauth-grant",
        ),
    )
    .await
    .expect("issue OAuth token");
    let (status, headers, body) = send(
        &fixture.router,
        rpc_request(Some(&oauth_token), None, list_tools()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["reason"], HTTP_MCP_API_KEY_REQUIRED);
    assert_eq!(
        headers[header::WWW_AUTHENTICATE],
        "Bearer error=\"invalid_token\""
    );

    let prefix = fixture
        .api_key
        .split_once('_')
        .map(|(prefix, _)| prefix)
        .expect("API keys carry a prefix");
    let unknown_key = format!("{prefix}_mcpfixtureunknownkey");
    let (status, headers, body) = send(
        &fixture.router,
        rpc_request(Some(&unknown_key), None, list_tools()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["reason"], HTTP_MCP_API_KEY_INVALID);
    assert_eq!(
        headers[header::WWW_AUTHENTICATE],
        "Bearer error=\"invalid_token\""
    );

    let mut malformed = rpc_request(None, None, list_tools());
    malformed.headers_mut().insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("Basic abc def"),
    );
    let (status, headers, body) = send(&fixture.router, malformed).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["reason"], HTTP_AUTHORIZATION_HEADER_INVALID);
    assert_eq!(
        headers[header::WWW_AUTHENTICATE],
        "Bearer error=\"invalid_request\""
    );
}

#[tokio::test]
async fn mcp_refuses_a_revoked_key_and_an_expired_key() {
    let fixture = fixture().await;
    let doomed = within(
        "API key",
        fixture.context.app.create_api_key(
            &fixture.admin,
            scryer_application::CreateApiKey {
                label: "mcp-fixture-revoked".into(),
                expiry: scryer_application::ApiKeyExpiryPreset::Never,
            },
        ),
    )
    .await
    .expect("create API key");
    let (status, _, body) = send(
        &fixture.router,
        rpc_request(Some(&doomed.raw_key), None, list_tools()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        within(
            "revoke",
            fixture
                .context
                .app
                .revoke_api_key(&fixture.admin, &doomed.api_key.id),
        )
        .await
        .expect("revoke API key")
    );
    let (status, headers, body) = send(
        &fixture.router,
        rpc_request(Some(&doomed.raw_key), None, list_tools()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["reason"], HTTP_MCP_API_KEY_INVALID);
    assert_eq!(
        headers[header::WWW_AUTHENTICATE],
        "Bearer error=\"invalid_token\""
    );

    let expired = "ska_AQEBAQEBAQEBAQEBAQEBAQ.AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE";
    within(
        "seed expired key",
        fixture.context.app.sync_development_api_keys(vec![
            scryer_application::DevelopmentApiKeySeed {
                username: fixture.admin.username.clone(),
                label: "mcp-fixture-expired".into(),
                raw_key: expired.into(),
                expires_at: Some(chrono::Utc::now() - chrono::Duration::days(1)),
            },
        ]),
    )
    .await
    .expect("seed expired key");
    let (status, _, body) = send(
        &fixture.router,
        rpc_request(Some(expired), None, list_tools()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["reason"], HTTP_MCP_API_KEY_INVALID);
}

#[tokio::test]
async fn mcp_refuses_a_foreign_origin_and_serves_a_missing_or_same_origin() {
    let fixture = fixture().await;
    let key = fixture.api_key.clone();

    let (status, _, body) = send(
        &fixture.router,
        rpc_request(
            Some(&key),
            Some("https://elsewhere.example.test"),
            list_tools(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["reason"], HTTP_MCP_ORIGIN_NOT_ALLOWED);

    // An Origin is checked before credentials, so a foreign page learns
    // nothing about which keys exist.
    let (status, _, body) = send(
        &fixture.router,
        rpc_request(None, Some("https://elsewhere.example.test"), list_tools()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["reason"], HTTP_MCP_ORIGIN_NOT_ALLOWED);

    let (status, _, body) =
        send(&fixture.router, rpc_request(Some(&key), None, list_tools())).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, _, body) = send(
        &fixture.router,
        rpc_request(Some(&key), Some(&format!("http://{HOST}")), list_tools()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn mcp_refuses_while_initial_setup_is_pending() {
    let fixture = fixture().await;
    let mut request = rpc_request(Some(&fixture.api_key), None, list_tools());
    request
        .extensions_mut()
        .insert(crate::initial_auth::SetupAdmission(Some(
            scryer_application::initial_auth::SetupState::Choice,
        )));
    let (status, _, body) = send(&fixture.router, request).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        body["errors"][0]["extensions"]["reason"],
        scryer_application::initial_auth::refusal::REQUIRED
    );
}

#[tokio::test]
async fn mcp_refuses_during_restore_maintenance() {
    let fixture = fixture().await;
    fixture
        .context
        .app
        .restore_coordinator()
        .start(
            fixture.context.app_data_dir.path(),
            "fixture",
            scryer_application::RestoreJobKind::Apply,
        )
        .expect("start restore");
    let (status, headers, body) = send(
        &fixture.router,
        rpc_request(Some(&fixture.api_key), None, call_tool("whoami", json!({}))),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(
        body["errors"][0]["extensions"]["reason"],
        scryer_application::restore_refusal::RESTORE_MAINTENANCE_ACTIVE
    );
    assert!(headers.get(header::WWW_AUTHENTICATE).is_none());
}

#[tokio::test]
async fn mcp_refuses_an_oversized_body_before_admission() {
    let fixture = fixture().await;
    let mut request = rpc_request(Some(&fixture.api_key), None, Value::Null);
    *request.body_mut() = Body::from(vec![b' '; MCP_MAX_REQUEST_BODY_BYTES + 1]);
    let (status, headers, body) = send(&fixture.router, request).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert_eq!(body["code"], "PAYLOAD_TOO_LARGE");
    assert_eq!(body["reason"], HTTP_MCP_REQUEST_TOO_LARGE);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn a_cancelled_call_stops_before_its_restore_lease_is_released() {
    let fixture = fixture().await;
    let probe = server::ExecutionProbe::default();
    let mut request = rpc_request(Some(&fixture.api_key), None, call_tool("whoami", json!({})));
    request.extensions_mut().insert(probe.clone());
    let entered = probe.entered.notified();
    tokio::pin!(entered);
    entered.as_mut().enable();
    let router = fixture.router.clone();
    let call = tokio::spawn(async move { router.oneshot(request).await });
    within("the tool call to start", entered).await;
    assert!(!probe.dropped.load(std::sync::atomic::Ordering::SeqCst));

    // The client goes away mid-call.
    call.abort();
    let _ = within("the aborted call", call).await;

    // A restore waits for every admitted request. When it gets through, the
    // tool's work must already be gone: a lease released while the work
    // still ran would let the restore start under it.
    let deadline = scryer_application::RestoreDeadline::with_budget(BOUND);
    within(
        "restore drain",
        fixture.context.app.restore_coordinator().drain(&deadline),
    )
    .await
    .expect("drain");
    assert!(probe.dropped.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn mcp_serves_whoami_and_lists_every_tool_in_order() {
    let fixture = fixture().await;
    let key = fixture.api_key.clone();

    let (status, headers, body) =
        send(&fixture.router, rpc_request(Some(&key), None, initialize())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(headers[header::CONTENT_TYPE], "application/json");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    // Stateless: nothing is kept between requests, so no session is issued.
    assert!(headers.get("mcp-session-id").is_none());
    let result = &body["result"];
    assert_eq!(result["serverInfo"]["name"], "scryer");
    assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(result["capabilities"]["tools"].is_object());
    assert!(result["capabilities"]["tools"]["listChanged"].is_null());
    assert!(result["capabilities"]["prompts"].is_null());
    assert!(result["capabilities"]["resources"].is_null());
    assert!(
        result["instructions"]
            .as_str()
            .is_some_and(|text| text.len() < 700)
    );

    for _ in 0..2 {
        let (status, _, body) =
            send(&fixture.router, rpc_request(Some(&key), None, list_tools())).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let tools = body["result"]["tools"].as_array().expect("tools");
        let names = tools
            .iter()
            .map(|tool| tool["name"].as_str().expect("name"))
            .collect::<Vec<_>>();
        assert_eq!(names, TOOL_NAMES);
        for tool in tools {
            let name = &tool["name"];
            assert!(tool["title"].is_string(), "{name}");
            assert!(tool["description"].is_string(), "{name}");
            assert_eq!(tool["annotations"]["readOnlyHint"], true, "{name}");
            assert_eq!(tool["annotations"]["openWorldHint"], false, "{name}");
            let schema = &tool["inputSchema"];
            assert_eq!(schema["type"], "object", "{name}");
            assert_eq!(schema["additionalProperties"], false, "{name}");
            assert!(schema["properties"].is_object(), "{name}");
        }
        assert_eq!(
            tools[0]["inputSchema"],
            json!({"type": "object", "properties": {}, "additionalProperties": false})
        );
    }

    let body = call(&fixture, &key, "whoami", json!({})).await;
    let content = success("whoami", &body);
    assert_eq!(content["me"]["username"], "admin", "{body}");
    assert!(content["me"]["appPermissions"].is_array(), "{body}");
}

#[tokio::test]
async fn mcp_refuses_unknown_tools_and_bad_arguments_without_echoing_them() {
    let fixture = fixture().await;
    let key = fixture.api_key.clone();

    let (_, _, body) = send(
        &fixture.router,
        rpc_request(
            Some(&key),
            None,
            call_tool("mcp-fixture-not-a-tool", json!({})),
        ),
    )
    .await;
    assert_eq!(protocol_error_reason(&body), HTTP_MCP_TOOL_UNKNOWN);
    assert!(
        !body.to_string().contains("mcp-fixture-not-a-tool"),
        "{body}"
    );

    for arguments in [
        json!({"mcpfixtureextra": 1}),
        json!({"query": 5}),
        json!({"query": "Fixture", "limit": 11}),
        json!({"query": "Fixture", "limit": "5"}),
        json!({"query": "x".repeat(201)}),
        json!({}),
    ] {
        let (_, _, body) = send(
            &fixture.router,
            rpc_request(
                Some(&key),
                None,
                call_tool("find_titles", arguments.clone()),
            ),
        )
        .await;
        assert_eq!(
            protocol_error_reason(&body),
            HTTP_MCP_ARGUMENTS_INVALID,
            "{arguments}"
        );
        assert!(!body.to_string().contains("mcpfixtureextra"), "{body}");
    }
    let (_, _, body) = send(
        &fixture.router,
        rpc_request(
            Some(&key),
            None,
            call_tool("whoami", json!({"user": "someone"})),
        ),
    )
    .await;
    assert_eq!(protocol_error_reason(&body), HTTP_MCP_ARGUMENTS_INVALID);
    let (_, _, body) = send(
        &fixture.router,
        rpc_request(
            Some(&key),
            None,
            call_tool(
                "get_title_history",
                json!({"title_id": "t", "event_types": ["NOT_AN_EVENT"]}),
            ),
        ),
    )
    .await;
    assert_eq!(protocol_error_reason(&body), HTTP_MCP_ARGUMENTS_INVALID);
    let (_, _, body) = send(
        &fixture.router,
        rpc_request(
            Some(&key),
            None,
            call_tool("read_diagnostics_section", json!({"section": "metrics"})),
        ),
    )
    .await;
    assert_eq!(protocol_error_reason(&body), HTTP_MCP_ARGUMENTS_INVALID);
    let (_, _, body) = send(
        &fixture.router,
        rpc_request(
            Some(&key),
            None,
            call_tool("search_service_logs", json!({"before": "not-a-cursor"})),
        ),
    )
    .await;
    assert_eq!(protocol_error_reason(&body), HTTP_MCP_ARGUMENTS_INVALID);
}

#[tokio::test]
async fn every_tool_answers_an_administrator_within_the_size_budget() {
    let fixture = fixture().await;
    let key = fixture.api_key.clone();
    let title_id =
        seed_title_with_decisions(&fixture.context, &["rejected", "accepted", "rejected"]).await;

    for (name, arguments) in minimal_calls(&title_id) {
        let body = call(&fixture, &key, name, arguments).await;
        success(name, &body);
    }

    let found = success(
        "find_titles",
        &call(
            &fixture,
            &key,
            "find_titles",
            json!({"query": "Fixture Probe"}),
        )
        .await,
    );
    assert!(
        found["total_count"]
            .as_i64()
            .is_some_and(|count| count >= 1),
        "{found}"
    );
    assert!(
        found["items"]
            .as_array()
            .is_some_and(|items| items.iter().any(|item| item["id"] == title_id.as_str())),
        "{found}"
    );

    let decisions = success(
        "get_title_decisions",
        &call(
            &fixture,
            &key,
            "get_title_decisions",
            json!({"title_id": title_id}),
        )
        .await,
    );
    assert_eq!(decisions["tally_covers"], "newest_25", "{decisions}");
    assert_eq!(decisions["returned"], 3, "{decisions}");

    // Every section, through the section names the tool declares.
    let (_, _, listed) = send(&fixture.router, rpc_request(Some(&key), None, list_tools())).await;
    let sections = listed["result"]["tools"]
        .as_array()
        .and_then(|tools| {
            tools
                .iter()
                .find(|tool| tool["name"] == "read_diagnostics_section")
        })
        .and_then(|tool| tool["inputSchema"]["properties"]["section"]["enum"].as_array())
        .cloned()
        .expect("section names");
    assert!(sections.len() > 10, "{sections:?}");
    for section in sections {
        let section = section.as_str().expect("section name");
        let arguments = if section == "metrics" {
            json!({"section": section, "metric_families": ["scryer_fixture_family"]})
        } else {
            json!({"section": section})
        };
        let body = call(&fixture, &key, "read_diagnostics_section", arguments).await;
        let content = success(section, &body);
        if section == "metrics" {
            // The fixture host records no metrics.
            assert_eq!(content["enabled"], false, "{content}");
        }
    }
}

#[tokio::test]
async fn release_decision_tally_counts_only_the_newest_sample() {
    let fixture = fixture().await;
    seed_title_with_decisions(&fixture.context, &["rejected", "accepted", "accepted"]).await;
    let tally = within(
        "tally",
        fixture
            .context
            .app
            .tally_recent_release_decision_codes(&fixture.admin, 2),
    )
    .await
    .expect("tally");
    assert_eq!(tally.total_rows, 3);
    assert_eq!(tally.sampled, 2);
    assert_eq!(tally.by_code, vec![("accepted".to_string(), 2)]);
}

#[tokio::test]
async fn tools_refuse_an_account_without_the_permission_they_need() {
    let fixture = fixture().await;
    let limited = within(
        "create limited user",
        fixture.context.app.create_user(
            &fixture.admin,
            "mcp-fixture-limited".into(),
            "mcp-fixture-limited-password".into(),
            AppPermissionMask::NONE,
            Vec::new(),
        ),
    )
    .await
    .expect("create limited user");
    let limited_key = create_key(&fixture.context, &limited).await;
    let title_id = seed_title_with_decisions(&fixture.context, &["rejected"]).await;

    let mut refused = Vec::new();
    for (name, arguments) in minimal_calls(&title_id) {
        let body = call(&fixture, &limited_key, name, arguments).await;
        if body["result"]["isError"] == true {
            let error = tool_error(name, &body);
            if DIAGNOSTICS_TOOLS.contains(&name) {
                assert_eq!(error["code"], "UNAUTHORIZED", "{name}: {body}");
                assert_eq!(
                    error["reason"], HTTP_DIAGNOSTICS_PERMISSION_DENIED,
                    "{name}: {body}"
                );
            }
            refused.push(name);
        } else {
            success(name, &body);
        }
    }
    for name in DIAGNOSTICS_TOOLS {
        assert!(
            refused.contains(name),
            "{name} was not refused: {refused:?}"
        );
    }
    // The account's own identity is always readable.
    assert!(!refused.contains(&"whoami"), "{refused:?}");
}

#[tokio::test]
async fn the_per_user_rate_limit_applies_inside_tool_calls() {
    let fixture = fixture_with(Options {
        auth_runtime: Some(form_login_runtime()),
        rate_limiter: Some(ScryerRateLimiter::for_api_test(1)),
        ..Options::default()
    })
    .await;
    let key = fixture.api_key.clone();
    success("whoami", &call(&fixture, &key, "whoami", json!({})).await);
    // A GraphQL tool and an application-read tool draw on the same bucket.
    for name in ["whoami", "list_download_queue", "get_diagnostics_overview"] {
        let body = call(&fixture, &key, name, json!({})).await;
        let error = tool_error(name, &body);
        assert_eq!(error["code"], "RATE_LIMITED", "{name}: {body}");
    }
}

#[tokio::test]
async fn one_tool_call_is_one_rate_limit_charge_whatever_it_runs() {
    let fixture = fixture_with(Options {
        auth_runtime: Some(form_login_runtime()),
        rate_limiter: Some(ScryerRateLimiter::for_api_test(1)),
        ..Options::default()
    })
    .await;
    let key = fixture.api_key.clone();
    // The stats variant runs two GraphQL operations on the one unit.
    let body = call(
        &fixture,
        &key,
        "list_indexers",
        json!({"include_stats": true}),
    )
    .await;
    success("list_indexers", &body);
    let body = call(&fixture, &key, "list_indexers", json!({})).await;
    assert_eq!(
        tool_error("list_indexers", &body)["code"],
        "RATE_LIMITED",
        "{body}"
    );
}

#[tokio::test]
async fn list_job_runs_refuses_arguments_that_do_not_apply() {
    let fixture = fixture().await;
    let key = fixture.api_key.clone();
    let tool = tools::tool(tools::find("list_job_runs").expect("list_job_runs"));
    let job_key = tool.input_schema["properties"]["job_key"]["enum"][0]
        .as_str()
        .expect("a job key")
        .to_string();
    for (arguments, argument) in [
        (json!({"job_key": job_key, "offset": 0}), "offset"),
        (json!({"include_summary": true}), "include_summary"),
    ] {
        let (_, _, body) = send(
            &fixture.router,
            rpc_request(Some(&key), None, call_tool("list_job_runs", arguments)),
        )
        .await;
        assert_eq!(protocol_error_reason(&body), HTTP_MCP_ARGUMENTS_INVALID);
        assert_eq!(body["error"]["data"]["argument"], argument, "{body}");
    }
    // The same arguments where they apply are served.
    success(
        "list_job_runs",
        &call(&fixture, &key, "list_job_runs", json!({"offset": 0})).await,
    );
    success(
        "list_job_runs",
        &call(
            &fixture,
            &key,
            "list_job_runs",
            json!({"job_key": job_key, "include_summary": true}),
        )
        .await,
    );
}

#[tokio::test]
async fn the_overview_carries_no_registered_host_value() {
    let fixture = fixture().await;
    let key = fixture.api_key.clone();
    let body = call(&fixture, &key, "get_diagnostics_overview", json!({})).await;
    let overview = success("get_diagnostics_overview", &body);
    // The database directory is a path the harvest registers.
    let data_dir = fixture.data_dir.path().display().to_string();
    assert!(!body.to_string().contains(&data_dir), "{body}");
    assert!(
        overview["host_sections"]
            .as_array()
            .is_some_and(|sections| sections.iter().all(Value::is_string)),
        "{overview}"
    );
    // The section tool still serves the host facts, sanitized.
    let body = call(
        &fixture,
        &key,
        "read_diagnostics_section",
        json!({"section": "installation"}),
    )
    .await;
    success("read_diagnostics_section", &body);
    assert!(!body.to_string().contains(&data_dir), "{body}");
}

#[tokio::test]
async fn the_api_explorer_header_has_no_effect_on_mcp() {
    let fixture = fixture().await;
    let key = fixture.api_key.clone();
    let plain = call(&fixture, &key, "whoami", json!({})).await;
    for mode in ["oauth", "api-key", "mcp-fixture-unknown-mode"] {
        let mut request = rpc_request(Some(&key), None, call_tool("whoami", json!({})));
        request
            .headers_mut()
            .insert("x-scryer-api-explorer-mode", HeaderValue::from_static(mode));
        let (status, _, body) = send(&fixture.router, request).await;
        assert_eq!(status, StatusCode::OK, "{mode}: {body}");
        assert_eq!(
            success("whoami", &body),
            success("whoami", &plain),
            "{mode}"
        );

        // The OAuth explorer mode would drop the system settings permission.
        let mut request = rpc_request(
            Some(&key),
            None,
            call_tool("get_diagnostics_overview", json!({})),
        );
        request
            .headers_mut()
            .insert("x-scryer-api-explorer-mode", HeaderValue::from_static(mode));
        let (status, _, body) = send(&fixture.router, request).await;
        assert_eq!(status, StatusCode::OK, "{mode}: {body}");
        success("get_diagnostics_overview", &body);
    }
}

fn request_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": {"name": "mcp-fixture-client", "version": "0.0.1"},
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

#[tokio::test]
async fn mcp_serves_the_per_request_protocol() {
    let fixture = fixture().await;
    let key = fixture.api_key.clone();

    let mut request = rpc_request_at(
        "2026-07-28",
        Some(&key),
        None,
        json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "server/discover",
            "params": {"_meta": request_meta()}
        }),
    );
    request
        .headers_mut()
        .insert("mcp-method", HeaderValue::from_static("server/discover"));
    let (status, _, body) = send(&fixture.router, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["result"].is_object(), "{body}");
    assert!(
        body["result"]["capabilities"]["tools"].is_object(),
        "{body}"
    );

    let call_body = json!({
        "jsonrpc": "2.0",
        "id": 8,
        "method": "tools/call",
        "params": {"name": "whoami", "arguments": {}, "_meta": request_meta()}
    });
    let mut request = rpc_request_at("2026-07-28", Some(&key), None, call_body.clone());
    request
        .headers_mut()
        .insert("mcp-method", HeaderValue::from_static("tools/call"));
    request
        .headers_mut()
        .insert("mcp-name", HeaderValue::from_static("whoami"));
    let (status, _, body) = send(&fixture.router, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(success("whoami", &body)["me"]["username"], "admin");

    // The headers must name the call; without them rmcp refuses it.
    let mut request = rpc_request_at("2026-07-28", Some(&key), None, call_body);
    request
        .headers_mut()
        .insert("mcp-method", HeaderValue::from_static("tools/call"));
    let (status, _, body) = send(&fixture.router, request).await;
    assert!(
        status != StatusCode::OK || body.get("error").is_some(),
        "{status}: {body}"
    );
}

fn write_log(logs: &LogRingBuffer, line: &str) {
    let mut writer = logs.clone();
    writer
        .write_all(format!("{line}\n").as_bytes())
        .expect("write log line");
}

#[tokio::test]
async fn log_search_is_bounded_and_sanitized() {
    let fixture = fixture().await;
    let key = fixture.api_key.clone();
    for index in 0..150 {
        write_log(&fixture.logs, &format!("mcp fixture filler line {index}"));
    }
    let data_path = fixture
        .data_dir
        .path()
        .join("media")
        .join("fixture-file.mkv");
    write_log(
        &fixture.logs,
        &format!(
            "mcp fixture secret line key={} path {}",
            fixture.api_key,
            data_path.display()
        ),
    );

    let body = call(
        &fixture,
        &key,
        "search_service_logs",
        json!({"pattern": "secret line", "source": "buffer"}),
    )
    .await;
    let found = success("search_service_logs", &body);
    assert_eq!(found["returned"], 1, "{found}");
    let line = found["lines"][0]["line"].as_str().expect("line");
    assert!(line.contains("secret line"), "{line}");
    assert!(!line.contains(&fixture.api_key), "{line}");
    let data_dir = fixture.data_dir.path().display().to_string();
    assert!(!line.contains(&data_dir), "{line}");
    assert!(!body.to_string().contains(&fixture.api_key));

    // The line cap leaves a cursor, and the cursor pages back.
    let first = success(
        "search_service_logs",
        &call(
            &fixture,
            &key,
            "search_service_logs",
            json!({"pattern": "filler", "source": "buffer", "max_lines": 100}),
        )
        .await,
    );
    assert_eq!(first["returned"], 100, "{first}");
    let cursor = first["next_cursor"]
        .as_str()
        .expect("next cursor")
        .to_string();
    let second = success(
        "search_service_logs",
        &call(
            &fixture,
            &key,
            "search_service_logs",
            json!({"pattern": "filler", "source": "buffer", "max_lines": 100, "before": cursor}),
        )
        .await,
    );
    assert_eq!(second["returned"], 50, "{second}");
    assert!(second["next_cursor"].is_null(), "{second}");

    let body = call(
        &fixture,
        &key,
        "search_service_logs",
        json!({"pattern": "(unclosed"}),
    )
    .await;
    let error = tool_error("search_service_logs", &body);
    assert_eq!(error["code"], "BAD_USER_INPUT");
    assert_eq!(error["reason"], HTTP_MCP_LOG_PATTERN_INVALID);

    let (_, _, body) = send(
        &fixture.router,
        rpc_request(
            Some(&key),
            None,
            call_tool(
                "search_service_logs",
                json!({"pattern": "a".repeat(scryer_application::diagnostics::LOG_PATTERN_MAX_CHARS + 1)}),
            ),
        ),
    )
    .await;
    assert_eq!(protocol_error_reason(&body), HTTP_MCP_ARGUMENTS_INVALID);
}

#[tokio::test]
async fn log_search_stops_at_its_scan_budget_with_a_cursor_that_continues() {
    let lines = tools::service_logs::MAX_LINES_SCANNED + 50;
    let fixture = fixture_with(Options {
        auth_runtime: Some(form_login_runtime()),
        log_lines: Some(lines),
        ..Options::default()
    })
    .await;
    let key = fixture.api_key.clone();
    for index in 0..lines {
        write_log(&fixture.logs, &format!("mcp fixture budget line {index}"));
    }
    let arguments = json!({"pattern": "mcp-fixture-never-matches", "source": "buffer"});
    let first = success(
        "search_service_logs",
        &call(&fixture, &key, "search_service_logs", arguments.clone()).await,
    );
    assert_eq!(first["returned"], 0, "{first}");
    assert_eq!(first["scan_capped"], true, "{first}");
    let cursor = first["next_cursor"].as_str().expect("next cursor");
    let mut arguments = arguments;
    arguments["before"] = json!(cursor);
    let second = success(
        "search_service_logs",
        &call(&fixture, &key, "search_service_logs", arguments).await,
    );
    assert_eq!(second["returned"], 0, "{second}");
    assert_eq!(second["scan_capped"], false, "{second}");
    assert!(second["next_cursor"].is_null(), "{second}");
}

#[test]
fn graphql_errors_become_tool_errors_built_from_extensions() {
    let mut extensions = async_graphql::ErrorExtensionValues::default();
    extensions.set("code", "RATE_LIMITED");
    extensions.set("reason", "FIXTURE_REASON");
    extensions.set("retryAfterSeconds", 7);
    let mut coded = async_graphql::ServerError::new("slow down: code NOT_THIS", None);
    coded.extensions = Some(extensions);
    let uncoded = async_graphql::ServerError::new("Unknown field \"nope\"", None);
    let response = async_graphql::Response::from_errors(vec![coded, uncoded]);

    let result = server::tool_result(response);
    assert_eq!(result.is_error, Some(true));
    let content = result.structured_content.expect("structured content");
    assert_eq!(
        content,
        json!({"errors": [
            {
                "code": "RATE_LIMITED",
                "reason": "FIXTURE_REASON",
                "message": "slow down: code NOT_THIS",
                "retryAfterSeconds": 7
            },
            {"code": HTTP_MCP_GRAPHQL_ERROR, "message": "Unknown field \"nope\""}
        ]})
    );
}

#[test]
fn a_partial_graphql_result_keeps_its_data_and_error_paths() {
    let data = async_graphql::Value::from_json(json!({
        "me": {"username": "fixture"},
        "settings": null
    }))
    .expect("data");
    let mut failed = async_graphql::ServerError::new("fixture failure", None);
    failed.path = vec![
        async_graphql::PathSegment::Field("settings".into()),
        async_graphql::PathSegment::Index(0),
    ];
    let mut response = async_graphql::Response::new(data);
    response.errors.push(failed);

    let result = server::tool_result(response);
    assert_eq!(result.is_error, Some(true));
    let content = result.structured_content.expect("structured content");
    assert_eq!(
        content,
        json!({
            "errors": [{
                "code": HTTP_MCP_GRAPHQL_ERROR,
                "message": "fixture failure",
                "path": ["settings", 0]
            }],
            "data": {"me": {"username": "fixture"}, "settings": null}
        })
    );

    // Data with nothing resolved is left out.
    let mut response =
        async_graphql::Response::new(async_graphql::Value::from_json(json!({"me": null})).unwrap());
    response
        .errors
        .push(async_graphql::ServerError::new("fixture failure", None));
    let content = server::tool_result(response)
        .structured_content
        .expect("structured content");
    assert!(content.get("data").is_none(), "{content}");
}
