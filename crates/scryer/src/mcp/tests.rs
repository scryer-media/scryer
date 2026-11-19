use super::*;
use crate::diagnostics_routes::DiagnosticsProcessFacts;
use crate::error_reasons::{
    HTTP_DIAGNOSTICS_PERMISSION_DENIED, HTTP_MCP_ARGUMENTS_INVALID, HTTP_MCP_GRAPHQL_ERROR,
    HTTP_MCP_LOG_PATTERN_INVALID, HTTP_MCP_OPERATION_AMBIGUOUS, HTTP_MCP_OPERATION_KIND_MISMATCH,
    HTTP_MCP_OPERATION_UNKNOWN, HTTP_MCP_RESULT_TOO_LARGE, HTTP_MCP_SKILL_UNKNOWN,
    HTTP_MCP_TOOL_UNKNOWN,
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

/// Awaits `future`, failing the test instead of hanging it.
async fn within<F: Future>(what: &str, future: F) -> F::Output {
    tokio::time::timeout(BOUND, future)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

struct Fixture {
    context: common::TestContext,
    router: Router,
    /// `/graphql` over the same auth state, to compare refusals with.
    graphql: Router,
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
    let auth = auth_state(&context, auth_runtime, rate_limiter);
    let graphql = Router::new()
        .route("/graphql", post(crate::middleware::graphql_handler))
        .with_state(auth.clone());
    let state = McpState::new(auth, host);
    let router = Router::new()
        .route(MCP_PATH, post(mcp_handler))
        .with_state(state);
    Fixture {
        context,
        router,
        graphql,
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

/// A tool result's value: the JSON object in its one text block. The
/// result carries it once, with no structured copy.
fn result_value(name: &str, body: &Value) -> Value {
    let result = &body["result"];
    assert!(result.get("structuredContent").is_none(), "{name}: {body}");
    let content = result["content"].as_array().expect("content");
    assert_eq!(content.len(), 1, "{name}: {body}");
    assert_eq!(content[0]["type"], "text", "{name}: {body}");
    let text = content[0]["text"].as_str().expect("text");
    let value: Value = serde_json::from_str(text).expect("JSON text");
    assert!(value.is_object(), "{name}: {body}");
    // Compact: serialized without whitespace.
    assert_eq!(text, value.to_string(), "{name}");
    value
}

/// A successful tool result's value.
fn success(name: &str, body: &Value) -> Value {
    assert_eq!(body["result"]["isError"], false, "{name}: {body}");
    result_value(name, body)
}

/// A tool error's first error entry.
fn tool_error(name: &str, body: &Value) -> Value {
    assert_eq!(body["result"]["isError"], true, "{name}: {body}");
    let error = result_value(name, body)["errors"][0].clone();
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
    assert!(second.get("scan_capped").is_none(), "{second}");
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

    let (content, failed) = server::graphql_value(response);
    assert!(failed);
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
    assert!(result.structured_content.is_none());
    let [block] = result.content.as_slice() else {
        panic!("one content block");
    };
    let content: Value =
        serde_json::from_str(&block.as_text().expect("a text block").text).expect("JSON text");
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
    let (content, _) = server::graphql_value(response);
    assert!(content.get("data").is_none(), "{content}");
}

/// Seeds the quality and routing settings definitions and the built-in
/// profiles, with the default profile as the global one, as a configured
/// instance has them.
async fn seed_quality_profiles(fixture: &Fixture) {
    let seed = |key_name: &str, data_type: &str, default_value_json: &str| {
        scryer_infrastructure_sql::types::SettingDefinitionSeed {
            category: "media".into(),
            scope: "system".into(),
            key_name: key_name.into(),
            data_type: data_type.into(),
            default_value_json: default_value_json.into(),
            is_sensitive: false,
            validation_json: None,
        }
    };
    within(
        "seed quality settings",
        fixture
            .context
            .settings_store
            .batch_ensure_setting_definitions(vec![
                seed("quality.profiles", "json", "[]"),
                seed("quality.profile_id", "string", "\"1080p\""),
                seed("quality.scoring_persona", "string", "\"Balanced\""),
                seed("indexer.routing", "string", "{}"),
                seed("download_client.routing", "string", "{}"),
                seed("nzbget.client_routing", "string", "{}"),
            ]),
    )
    .await
    .expect("seed quality settings");
    within(
        "save quality profiles",
        fixture.context.app.save_quality_profile_settings(
            &fixture.admin,
            scryer_application::SaveQualityProfileSettings {
                profiles: vec![
                    scryer_application::builtin_default_quality_profile(),
                    scryer_application::builtin_4k_profile(),
                ],
                replace_existing: true,
                global_profile_id: Some(
                    scryer_application::BUILTIN_DEFAULT_QUALITY_PROFILE_ID.to_string(),
                ),
                category_selections: Vec::new(),
                global_scoring_persona: None,
                category_persona_selections: Vec::new(),
            },
        ),
    )
    .await
    .expect("save quality profiles");
}

/// Posts one operation to `/graphql` with `key` and returns the first
/// error's extensions.
async fn graphql_refusal(fixture: &Fixture, key: &str, document: &str, variables: Value) -> Value {
    let body = graphql_body(fixture, key, document, variables).await;
    let extensions = body["errors"][0]["extensions"].clone();
    assert!(extensions["code"].is_string(), "{body}");
    extensions
}

/// Posts one operation to `/graphql` with `key` and returns the response.
async fn graphql_body(fixture: &Fixture, key: &str, document: &str, variables: Value) -> Value {
    let mut request = Request::builder()
        .method("POST")
        .uri("/graphql")
        .header(header::HOST, HOST)
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {key}")).expect("authorization"),
        )
        .body(Body::from(
            json!({"query": document, "variables": variables}).to_string(),
        ))
        .expect("GraphQL request");
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from((Ipv4Addr::LOCALHOST, 3000))));
    let response = within("GraphQL response", fixture.graphql.clone().oneshot(request))
        .await
        .expect("GraphQL response");
    let bytes = within(
        "GraphQL body",
        axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024),
    )
    .await
    .expect("GraphQL body");
    serde_json::from_slice(&bytes).expect("GraphQL JSON")
}

fn fixture_indexer(id: &str) -> scryer_domain::IndexerConfig {
    let now = chrono::Utc::now();
    scryer_domain::IndexerConfig {
        id: id.to_string(),
        name: format!("Fixture Indexer {id}"),
        provider_type: "newznab".into(),
        base_url: "http://indexer.fixture.invalid".into(),
        api_key_encrypted: None,
        rate_limit_seconds: None,
        rate_limit_burst: None,
        max_queries_per_minute: None,
        max_requests_per_month: None,
        max_grabs_per_month: None,
        disabled_until: None,
        is_enabled: false,
        enable_interactive_search: true,
        enable_auto_search: true,
        proxy_config_id: None,
        download_client_id: None,
        seeding_profile_id: None,
        managed_parent_config_id: None,
        managed_child_key: None,
        managed_metadata_json: None,
        caps_snapshot_json: None,
        last_health_status: None,
        last_error_message: None,
        last_error_code: None,
        last_error_at: None,
        config_json: None,
        created_at: now,
        updated_at: now,
    }
}

fn fixture_client(id: &str, priority: i64) -> scryer_domain::DownloadClientConfig {
    let now = chrono::Utc::now();
    scryer_domain::DownloadClientConfig {
        id: id.to_string(),
        name: format!("Fixture Client {id}"),
        client_type: "nzbget".into(),
        config_json: r#"{"base_url":"http://client.fixture.invalid"}"#.into(),
        client_priority: priority,
        is_enabled: false,
        status: scryer_domain::DownloadClientStatus::Healthy,
        last_error: None,
        last_seen_at: None,
        proxy_config_id: None,
        created_at: now,
        updated_at: now,
    }
}

/// Seeds disabled indexer and download-client configs straight into their
/// stores, so nothing probes a remote service.
async fn seed_integrations(context: &common::TestContext, indexers: &[&str], clients: &[&str]) {
    use scryer_application::{DownloadClientConfigRepository, IndexerConfigRepository};
    let indexer_store =
        scryer_infrastructure_acquisition::indexers::config_store::IndexerConfigStore::new(
            context.db.datastore(),
            context.db.encryption_key_state(),
        );
    for id in indexers {
        within("seed indexer", indexer_store.create(fixture_indexer(id)))
            .await
            .expect("seed indexer");
    }
    let client_store =
        scryer_infrastructure_acquisition::downloads::config_store::DownloadClientConfigStore::new(
            context.db.datastore(),
            context.db.encryption_key_state(),
        );
    for (index, id) in clients.iter().enumerate() {
        within(
            "seed download client",
            client_store.create(fixture_client(id, index as i64)),
        )
        .await
        .expect("seed download client");
    }
}

const TOOL_NAMES: &[&str] = &[
    "whoami",
    "list_operations",
    "describe_operation",
    "run_query",
    "run_mutation",
    "read_diagnostics_section",
    "search_service_logs",
];

/// A catalog operation's document, as the run tools send it.
fn catalog_text(name: &str) -> &'static str {
    catalog::CATALOG.get(name).expect("catalog operation").text
}

async fn limited_key(fixture: &Fixture) -> String {
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
    create_key(&fixture.context, &limited).await
}

/// The indexer routing write the run tests save and read back.
fn routing_input() -> Value {
    json!({"input": {"scope": "MOVIE", "entries": [{
        "indexerId": "mcp-fixture-indexer",
        "enabled": true,
        "categories": ["2000"],
        "priority": 3,
    }]}})
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
    assert!(result["instructions"].is_string());

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
            assert!(tool.get("title").is_none(), "{name}");
            assert!(tool["description"].is_string(), "{name}");
            let annotations = &tool["annotations"];
            if *name == "run_mutation" {
                assert_eq!(annotations["readOnlyHint"], false, "{name}");
                assert_eq!(annotations["destructiveHint"], true, "{name}");
                assert_eq!(annotations["idempotentHint"], false, "{name}");
            } else {
                assert_eq!(annotations["readOnlyHint"], true, "{name}");
                assert!(annotations.get("destructiveHint").is_none(), "{name}");
            }
            let open_world = *name == "run_query" || *name == "run_mutation";
            assert_eq!(annotations["openWorldHint"], open_world, "{name}");
            let schema = &tool["inputSchema"];
            assert_eq!(schema["type"], "object", "{name}");
            assert!(schema["properties"].is_object(), "{name}");
        }
        assert_eq!(
            tools[0]["inputSchema"],
            json!({"type": "object", "properties": {}})
        );
    }

    let body = call(&fixture, &key, "whoami", json!({})).await;
    let content = success("whoami", &body);
    assert_eq!(content["me"]["username"], "admin", "{body}");
    assert!(content["me"]["appPermissions"].is_array(), "{body}");
}

#[tokio::test]
async fn the_tool_list_and_instructions_stay_within_their_token_budget() {
    let fixture = fixture().await;
    let key = fixture.api_key.clone();
    let (_, _, initialized) =
        send(&fixture.router, rpc_request(Some(&key), None, initialize())).await;
    let instructions = initialized["result"]["instructions"]
        .as_str()
        .expect("instructions")
        .len();
    let (_, _, listed) = send(&fixture.router, rpc_request(Some(&key), None, list_tools())).await;
    let tools = listed["result"].to_string().len();
    // Measured at 326 + 2,945 characters, about 820 tokens together.
    assert!(instructions + tools < 3_300, "{instructions} + {tools}");
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

    for (name, arguments) in [
        ("run_query", json!({"name": "Me", "mcpfixtureextra": 1})),
        ("run_query", json!({"name": 5})),
        ("run_query", json!({"name": "x".repeat(101)})),
        ("run_query", json!({"variables": {}})),
        ("run_query", json!({"name": "Me", "variables": [1]})),
        (
            "describe_operation",
            json!({"name": "Me", "document": "yes"}),
        ),
        ("list_operations", json!({"skill": 5})),
        ("whoami", json!({"user": "someone"})),
        ("read_diagnostics_section", json!({"section": "metrics"})),
        ("search_service_logs", json!({"before": "not-a-cursor"})),
    ] {
        let (_, _, body) = send(
            &fixture.router,
            rpc_request(Some(&key), None, call_tool(name, arguments.clone())),
        )
        .await;
        assert_eq!(
            protocol_error_reason(&body),
            HTTP_MCP_ARGUMENTS_INVALID,
            "{name} {arguments}"
        );
        assert!(!body.to_string().contains("mcpfixtureextra"), "{body}");
    }

    // A skill is a plain string: one with no served operation is a coded
    // refusal, and one that does not carry the name an unknown operation
    // naming the skills that do. Neither echoes the skill sent.
    let body = call(
        &fixture,
        &key,
        "list_operations",
        json!({"skill": "mcp-fixture-no-skill"}),
    )
    .await;
    let error = tool_error("list_operations", &body);
    assert_eq!(error["reason"], HTTP_MCP_SKILL_UNKNOWN, "{body}");
    assert_eq!(error["skills"], json!(catalog::CATALOG.skills()), "{body}");
    assert!(!body.to_string().contains("mcp-fixture-no-skill"), "{body}");

    let me_skills = catalog::CATALOG
        .variants("Me")
        .iter()
        .flat_map(|variant| variant.skills.iter().copied())
        .collect::<Vec<_>>();
    for name in ["run_query", "describe_operation"] {
        let body = call(
            &fixture,
            &key,
            name,
            json!({"name": "Me", "skill": "mcp-fixture-no-skill"}),
        )
        .await;
        let error = tool_error(name, &body);
        assert_eq!(error["reason"], HTTP_MCP_OPERATION_UNKNOWN, "{body}");
        assert_eq!(error["skills"], json!(me_skills), "{body}");
        assert!(!body.to_string().contains("mcp-fixture-no-skill"), "{body}");
    }
}

#[tokio::test]
async fn list_operations_groups_the_catalog_by_skill_and_filters_to_one() {
    let fixture = fixture().await;
    let key = fixture.api_key.clone();
    let all = success(
        "list_operations",
        &call(&fixture, &key, "list_operations", json!({})).await,
    );
    let groups = all.as_object().expect("skill groups");
    assert_eq!(
        groups.keys().map(String::as_str).collect::<Vec<_>>(),
        catalog::CATALOG.skills()
    );
    let mut listed = 0;
    for (skill, rows) in groups {
        for row in rows.as_array().expect("rows") {
            let row = row.as_array().expect("row");
            let catalog::Resolution::Found(operation) =
                catalog::CATALOG.resolve(row[0].as_str().expect("name"), Some(skill))
            else {
                panic!("{row:?} is not in the catalog under {skill}");
            };
            assert_eq!(row[1], operation.kind.token(), "{row:?}");
            assert!(row.len() == 2 || row[2].as_str().is_some_and(|p| !p.is_empty()));
            listed += 1;
        }
    }
    let memberships = catalog::CATALOG
        .operations()
        .iter()
        .map(|operation| operation.skills.len())
        .sum::<usize>();
    assert_eq!(listed, memberships);
    for denied in catalog::DENIED {
        assert!(
            !all.to_string().contains(&format!("\"{denied}\"")),
            "{denied}"
        );
    }

    let one = success(
        "list_operations",
        &call(
            &fixture,
            &key,
            "list_operations",
            json!({"skill": "scryer-configure-acquisition"}),
        )
        .await,
    );
    let groups = one.as_object().expect("one group");
    assert_eq!(groups.len(), 1, "{one}");
    assert_eq!(
        groups["scryer-configure-acquisition"],
        all["scryer-configure-acquisition"]
    );
    assert!(
        groups["scryer-configure-acquisition"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| row[0] == "IndexerRouting")),
        "{one}"
    );
}

#[tokio::test]
async fn describe_operation_gives_variables_and_the_types_they_use() {
    let fixture = fixture().await;
    let key = fixture.api_key.clone();

    let query = success(
        "describe_operation",
        &call(
            &fixture,
            &key,
            "describe_operation",
            json!({"name": "IndexerRouting"}),
        )
        .await,
    );
    assert_eq!(query["name"], "IndexerRouting");
    assert_eq!(query["kind"], "q");
    assert!(query["notes"].is_string(), "{query}");
    assert_eq!(query["variables"], json!({"scope": "ContentScopeValue!"}));
    let scopes = query["types"]["ContentScopeValue"]["enum"]
        .as_array()
        .expect("enum values");
    assert!(scopes.contains(&json!("MOVIE")), "{query}");
    assert!(query.get("document").is_none(), "{query}");

    let mutation = success(
        "describe_operation",
        &call(
            &fixture,
            &key,
            "describe_operation",
            json!({"name": "UpdateIndexerRouting", "document": true}),
        )
        .await,
    );
    assert_eq!(mutation["kind"], "m");
    assert_eq!(
        mutation["variables"],
        json!({"input": "UpdateIndexerRoutingInput!"})
    );
    let input = &mutation["types"]["UpdateIndexerRoutingInput"]["fields"];
    assert!(
        input["scope"].is_string() || input["scope"]["type"].is_string(),
        "{mutation}"
    );
    assert!(
        input["entries"].is_string() || input["entries"]["type"].is_string(),
        "{mutation}"
    );
    assert!(
        mutation["types"]["ContentScopeValue"]["enum"].is_array(),
        "{mutation}"
    );
    assert_eq!(mutation["document"], catalog_text("UpdateIndexerRouting"));
    for value in mutation["types"].as_object().expect("types").values() {
        for (field, shown) in value.as_object().expect("type") {
            assert!(!shown.is_null(), "{field}: {mutation}");
            assert_ne!(*shown, json!([]), "{field}: {mutation}");
            assert_ne!(*shown, json!({}), "{field}: {mutation}");
        }
    }

    let unknown = call(
        &fixture,
        &key,
        "describe_operation",
        json!({"name": "McpFixtureNoOperation"}),
    )
    .await;
    let error = tool_error("describe_operation", &unknown);
    assert_eq!(error["reason"], HTTP_MCP_OPERATION_UNKNOWN);
    assert!(error.get("skills").is_none(), "{unknown}");

    // The description with the most expanded types still fits one result:
    // its types stop at their byte budget and the rest are named.
    let mut largest = (0, String::new(), Value::Null);
    for operation in catalog::CATALOG.operations() {
        let body = call(
            &fixture,
            &key,
            "describe_operation",
            json!({"name": operation.name, "skill": operation.skills[0], "document": true}),
        )
        .await;
        let text = body["result"]["content"][0]["text"]
            .as_str()
            .expect("one text block");
        assert!(
            text.len() <= tools::RESULT_MAX_BYTES,
            "{} is {} bytes",
            operation.name,
            text.len()
        );
        let types = success("describe_operation", &body)["types"]
            .to_string()
            .len();
        if types > largest.0 {
            largest = (types, operation.name.clone(), body);
        }
    }
    let (types, name, body) = largest;
    assert!(
        types <= tools::RESULT_MAX_BYTES / 2,
        "{name} types are {types} bytes"
    );
    let described = success("describe_operation", &body);
    for unexpanded in described["unexpanded"].as_array().into_iter().flatten() {
        let unexpanded = unexpanded.as_str().expect("a type name");
        assert!(described["types"].get(unexpanded).is_none(), "{name}");
    }
}

#[tokio::test]
async fn run_query_serves_catalog_queries_and_refuses_with_codes() {
    let fixture = fixture().await;
    seed_quality_profiles(&fixture).await;
    seed_integrations(&fixture.context, &["mcp-fixture-indexer"], &[]).await;
    let key = fixture.api_key.clone();

    let body = call(
        &fixture,
        &key,
        "run_query",
        json!({"name": "IndexerRouting", "variables": {"scope": "MOVIE"}}),
    )
    .await;
    let content = success("run_query", &body);
    assert!(content["indexerRouting"].is_array(), "{body}");
    let me = success(
        "run_query",
        &call(&fixture, &key, "run_query", json!({"name": "Me"})).await,
    );
    assert_eq!(me["me"]["username"], "admin");

    // A key without the permission is refused with GraphQL's own code.
    let limited = limited_key(&fixture).await;
    let variables = json!({"scope": "MOVIE"});
    let body = call(
        &fixture,
        &limited,
        "run_query",
        json!({"name": "IndexerRouting", "variables": variables}),
    )
    .await;
    let error = tool_error("run_query", &body);
    let expected = graphql_refusal(
        &fixture,
        &limited,
        catalog_text("IndexerRouting"),
        variables,
    )
    .await;
    assert_eq!(error["code"], "UNAUTHORIZED", "{body}");
    assert_eq!(error["code"], expected["code"], "{body}");
    assert_eq!(error["reason"], expected["reason"], "{body}");

    let body = call(
        &fixture,
        &key,
        "run_query",
        json!({"name": "McpFixtureNoOperation"}),
    )
    .await;
    let error = tool_error("run_query", &body);
    assert_eq!(error["code"], "BAD_USER_INPUT");
    assert_eq!(error["reason"], HTTP_MCP_OPERATION_UNKNOWN);
    assert!(
        !body.to_string().contains("McpFixtureNoOperation"),
        "{body}"
    );

    let body = call(
        &fixture,
        &key,
        "run_query",
        json!({"name": "UpdateIndexerRouting", "variables": routing_input()}),
    )
    .await;
    let error = tool_error("run_query", &body);
    assert_eq!(error["reason"], HTTP_MCP_OPERATION_KIND_MISMATCH);
    assert_eq!(error["tool"], "run_mutation");
    let body = call(
        &fixture,
        &key,
        "run_mutation",
        json!({"name": "IndexerRouting", "variables": {"scope": "MOVIE"}}),
    )
    .await;
    let error = tool_error("run_mutation", &body);
    assert_eq!(error["reason"], HTTP_MCP_OPERATION_KIND_MISMATCH);
    assert_eq!(error["tool"], "run_query");
    // Neither refused call ran: the routing is still empty.
    let routing = success(
        "run_query",
        &call(
            &fixture,
            &key,
            "run_query",
            json!({"name": "IndexerRouting", "variables": {"scope": "MOVIE"}}),
        )
        .await,
    );
    assert!(
        routing["indexerRouting"]
            .as_array()
            .is_some_and(|entries| entries.iter().all(|entry| entry["priority"] != 3)),
        "{routing}"
    );

    // Variables reach GraphQL unchanged, and its validation error is coded.
    let variables = json!({"scope": "MCP_FIXTURE_SCOPE"});
    let body = call(
        &fixture,
        &key,
        "run_query",
        json!({"name": "IndexerRouting", "variables": variables}),
    )
    .await;
    let error = tool_error("run_query", &body);
    let expected = graphql_body(&fixture, &key, catalog_text("IndexerRouting"), variables).await;
    assert!(expected["errors"][0].is_object(), "{expected}");
    // GraphQL gives this validation error no code; the tool's own stands in.
    let expected_code = expected["errors"][0]["extensions"]["code"]
        .as_str()
        .unwrap_or(HTTP_MCP_GRAPHQL_ERROR);
    assert_eq!(error["code"], expected_code, "{body}");

    // Over the cap: refused with both sizes, never cut.
    let mut request = rpc_request(
        Some(&key),
        None,
        call_tool("run_query", json!({"name": "Me"})),
    );
    request.extensions_mut().insert(server::ResultCap(16));
    let (status, _, body) = send(&fixture.router, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let error = tool_error("run_query", &body);
    assert_eq!(error["code"], "PAYLOAD_TOO_LARGE");
    assert_eq!(error["reason"], HTTP_MCP_RESULT_TOO_LARGE);
    assert_eq!(error["max_bytes"], 16);
    assert!(error["result_bytes"].as_u64().is_some_and(|size| size > 16));
    assert!(error.get("applied").is_none());
    assert!(result_value("run_query", &body).get("data").is_none());
    assert!(!body.to_string().contains("\"admin\""), "{body}");
}

#[tokio::test]
async fn run_mutation_saves_a_configuration_change_end_to_end() {
    let fixture = fixture().await;
    seed_quality_profiles(&fixture).await;
    seed_integrations(&fixture.context, &["mcp-fixture-indexer"], &[]).await;
    let key = fixture.api_key.clone();

    let body = call(
        &fixture,
        &key,
        "run_mutation",
        json!({"name": "UpdateIndexerRouting", "variables": routing_input()}),
    )
    .await;
    let saved = success("run_mutation", &body);
    assert_eq!(
        saved["updateIndexerRouting"]
            .as_array()
            .and_then(|entries| entries
                .iter()
                .find(|entry| entry["indexerId"] == "mcp-fixture-indexer"))
            .map(|entry| entry["priority"].clone()),
        Some(json!(3)),
        "{body}"
    );
    let read = success(
        "run_query",
        &call(
            &fixture,
            &key,
            "run_query",
            json!({"name": "IndexerRouting", "variables": {"scope": "MOVIE"}}),
        )
        .await,
    );
    let entry = read["indexerRouting"]
        .as_array()
        .and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry["indexerId"] == "mcp-fixture-indexer")
        })
        .cloned()
        .expect("saved entry");
    assert_eq!(entry["priority"], 3, "{read}");
    assert_eq!(entry["categories"], json!(["2000"]), "{read}");

    // A mutation over the cap still ran, and says so.
    let mut request = rpc_request(
        Some(&key),
        None,
        call_tool(
            "run_mutation",
            json!({"name": "UpdateIndexerRouting", "variables": routing_input()}),
        ),
    );
    request.extensions_mut().insert(server::ResultCap(16));
    let (_, _, body) = send(&fixture.router, request).await;
    let error = tool_error("run_mutation", &body);
    assert_eq!(error["reason"], HTTP_MCP_RESULT_TOO_LARGE);
    assert_eq!(error["applied"], true);
}

#[tokio::test]
async fn the_diagnostics_tools_refuse_an_account_without_system_settings() {
    let fixture = fixture().await;
    let limited = limited_key(&fixture).await;
    for (name, arguments) in [
        ("read_diagnostics_section", json!({"section": "build"})),
        ("search_service_logs", json!({})),
    ] {
        let body = call(&fixture, &limited, name, arguments).await;
        let error = tool_error(name, &body);
        assert_eq!(error["code"], "UNAUTHORIZED", "{name}: {body}");
        assert_eq!(
            error["reason"], HTTP_DIAGNOSTICS_PERMISSION_DENIED,
            "{name}: {body}"
        );
    }
    // The account's own identity is always readable.
    success(
        "whoami",
        &call(&fixture, &limited, "whoami", json!({})).await,
    );
}

#[tokio::test]
async fn every_diagnostics_section_answers_an_administrator() {
    let fixture = fixture().await;
    let key = fixture.api_key.clone();
    seed_title_with_decisions(&fixture.context, &["rejected", "accepted", "rejected"]).await;
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
async fn the_installation_section_carries_no_registered_host_value() {
    let fixture = fixture().await;
    let key = fixture.api_key.clone();
    // The database directory is a path the harvest registers.
    let data_dir = fixture.data_dir.path().display().to_string();
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
    for (name, arguments) in [
        ("whoami", json!({})),
        ("run_query", json!({"name": "Me"})),
        ("read_diagnostics_section", json!({"section": "build"})),
        ("list_operations", json!({})),
    ] {
        let body = call(&fixture, &key, name, arguments).await;
        let error = tool_error(name, &body);
        assert_eq!(error["code"], "RATE_LIMITED", "{name}: {body}");
    }
}

#[tokio::test]
async fn a_catalog_run_draws_on_its_operations_graphql_class() {
    let fixture = fixture_with(Options {
        auth_runtime: Some(form_login_runtime()),
        rate_limiter: Some(ScryerRateLimiter::for_class_test(1, 1, 1)),
        ..Options::default()
    })
    .await;
    let key = fixture.api_key.clone();
    // A search-class query with a required variable: sent without it, the
    // call is charged and GraphQL refuses it before any resolver runs.
    let search = catalog::CATALOG
        .operations()
        .iter()
        .find(|operation| {
            operation.kind == catalog::OperationKind::Query
                && operation.class == crate::rate_limit::GraphqlRateLimitClass::Search
                && operation.skills.len() == 1
                && catalog::CATALOG.variants(&operation.name).len() == 1
                && operation
                    .variables
                    .iter()
                    .any(|variable| variable.ty.ends_with('!') && variable.default.is_none())
        })
        .expect("a search-class catalog query with a required variable");
    let mutation = catalog::CATALOG
        .get("UpdateIndexerRouting")
        .expect("a catalog mutation");
    assert_eq!(
        mutation.class,
        crate::rate_limit::GraphqlRateLimitClass::Mutation
    );
    let runs = [
        ("run_query", json!({"name": search.name})),
        ("run_mutation", json!({"name": mutation.name})),
    ];
    for (tool, arguments) in &runs {
        let body = call(&fixture, &key, tool, arguments.clone()).await;
        assert_ne!(tool_error(tool, &body)["code"], "RATE_LIMITED", "{body}");
    }
    // Each has spent its own one-request bucket.
    for (tool, arguments) in &runs {
        let body = call(&fixture, &key, tool, arguments.clone()).await;
        assert_eq!(tool_error(tool, &body)["code"], "RATE_LIMITED", "{body}");
    }
    // The API bucket is untouched; an unknown name is charged to it.
    let body = call(
        &fixture,
        &key,
        "run_query",
        json!({"name": "McpFixtureNoOperation"}),
    )
    .await;
    assert_eq!(
        tool_error("run_query", &body)["reason"],
        HTTP_MCP_OPERATION_UNKNOWN,
        "{body}"
    );
    let body = call(&fixture, &key, "whoami", json!({})).await;
    assert_eq!(
        tool_error("whoami", &body)["code"],
        "RATE_LIMITED",
        "{body}"
    );
}

#[tokio::test]
async fn every_withheld_operation_is_unknown_to_the_run_tools() {
    let fixture = fixture().await;
    let key = fixture.api_key.clone();
    assert!(!catalog::CATALOG.withheld.is_empty());
    for name in &catalog::CATALOG.withheld {
        let body = call(&fixture, &key, "run_mutation", json!({"name": name})).await;
        assert_eq!(
            tool_error("run_mutation", &body)["reason"],
            HTTP_MCP_OPERATION_UNKNOWN,
            "{name}: {body}"
        );
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
    // The call runs GraphQL, which draws on the same bucket, yet a one-unit
    // budget serves it: the call is charged once, at its start.
    success(
        "run_query",
        &call(&fixture, &key, "run_query", json!({"name": "Me"})).await,
    );
    let body = call(&fixture, &key, "run_query", json!({"name": "Me"})).await;
    assert_eq!(
        tool_error("run_query", &body)["code"],
        "RATE_LIMITED",
        "{body}"
    );
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
            call_tool("read_diagnostics_section", json!({"section": "build"})),
        );
        request
            .headers_mut()
            .insert("x-scryer-api-explorer-mode", HeaderValue::from_static(mode));
        let (status, _, body) = send(&fixture.router, request).await;
        assert_eq!(status, StatusCode::OK, "{mode}: {body}");
        success("read_diagnostics_section", &body);
    }
}

#[tokio::test]
async fn config_step_up_refuses_api_key_writes_with_the_graphql_code() {
    let fixture = fixture().await;
    let key = fixture.api_key.clone();
    within(
        "seed step-up setting",
        fixture
            .context
            .settings_store
            .batch_ensure_setting_definitions(vec![
                scryer_infrastructure_sql::types::SettingDefinitionSeed {
                    category: "security".into(),
                    scope: "system".into(),
                    key_name: "auth.mfa.require_config_step_up".into(),
                    data_type: "boolean".into(),
                    default_value_json: "false".into(),
                    is_sensitive: false,
                    validation_json: None,
                },
            ]),
    )
    .await
    .expect("seed configuration MFA step-up setting");
    within(
        "enable step-up",
        fixture.context.settings_store.upsert_setting_value(
            "system",
            "auth.mfa.require_config_step_up",
            None,
            "true",
            "test",
            None,
        ),
    )
    .await
    .expect("enable configuration MFA step-up");
    // Step-up applies while form login is in effect, as it is for this
    // instance's GraphQL context.
    fixture
        .context
        .auth_runtime
        .apply_saved_security_settings(true, false);
    seed_quality_profiles(&fixture).await;
    seed_integrations(&fixture.context, &["mcp-fixture-indexer"], &[]).await;

    // The owner has no TOTP credential, so the write is refused for
    // enrollment, exactly as `/graphql` refuses it.
    let body = call(
        &fixture,
        &key,
        "run_mutation",
        json!({"name": "UpdateIndexerRouting", "variables": routing_input()}),
    )
    .await;
    let error = tool_error("run_mutation", &body);
    let expected = graphql_refusal(
        &fixture,
        &key,
        catalog_text("UpdateIndexerRouting"),
        routing_input(),
    )
    .await;
    assert_eq!(error["code"], "TOTP_ENROLLMENT_REQUIRED", "{body}");
    assert_eq!(error["code"], expected["code"], "{body}");
    assert_eq!(error["reason"], expected["reason"], "{body}");
    // Reads are not configuration changes.
    success(
        "run_query",
        &call(
            &fixture,
            &key,
            "run_query",
            json!({"name": "IndexerRouting", "variables": {"scope": "MOVIE"}}),
        )
        .await,
    );
}

#[tokio::test]
async fn a_name_that_differs_between_skills_needs_its_skill() {
    let fixture = fixture().await;
    seed_quality_profiles(&fixture).await;
    let key = fixture.api_key.clone();
    let name = "DelayProfiles";
    let skills = ["scryer-configure-acquisition", "scryer-migrate-from-arr"];
    assert!(catalog::CATALOG.varied.iter().any(|varied| varied == name));

    for tool in ["describe_operation", "run_query"] {
        let body = call(&fixture, &key, tool, json!({"name": name})).await;
        let error = tool_error(tool, &body);
        assert_eq!(error["code"], "BAD_USER_INPUT", "{body}");
        assert_eq!(error["reason"], HTTP_MCP_OPERATION_AMBIGUOUS, "{body}");
        assert_eq!(error["skills"], json!(skills), "{body}");

        // A skill that does not carry the name finds nothing, and the
        // refusal names the skills that do.
        let body = call(
            &fixture,
            &key,
            tool,
            json!({"name": name, "skill": "scryer-connect"}),
        )
        .await;
        let error = tool_error(tool, &body);
        assert_eq!(error["reason"], HTTP_MCP_OPERATION_UNKNOWN, "{body}");
        assert_eq!(error["skills"], json!(skills), "{body}");
    }
    for skill in skills {
        let described = success(
            "describe_operation",
            &call(
                &fixture,
                &key,
                "describe_operation",
                json!({"name": name, "skill": skill, "document": true}),
            )
            .await,
        );
        let catalog::Resolution::Found(variant) = catalog::CATALOG.resolve(name, Some(skill))
        else {
            panic!("{name} under {skill}");
        };
        assert_eq!(described["document"], variant.text, "{skill}");
        success(
            "run_query",
            &call(
                &fixture,
                &key,
                "run_query",
                json!({"name": name, "skill": skill}),
            )
            .await,
        );
    }
    // A name with one text resolves with or without a skill that carries it.
    success(
        "run_query",
        &call(
            &fixture,
            &key,
            "run_query",
            json!({"name": "Me", "skill": "scryer-connect"}),
        )
        .await,
    );
}
