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
    "list_rule_sets",
    "get_rule_set",
    "list_title_episodes",
    "validate_rule_set",
    "test_rule_set",
    "create_rule_set",
    "update_rule_set",
    "set_rule_set_enabled",
    "list_delay_profiles",
    "get_quality_profile_settings",
    "get_indexer_routing",
    "get_download_client_routing",
    "list_seeding_profiles",
    "list_indexer_provider_types",
    "list_download_client_provider_types",
    "upsert_delay_profile",
    "save_quality_profile_settings",
    "update_indexer_routing",
    "update_download_client_routing",
    "reorder_download_clients",
    "set_indexer_download_client_mapping",
];

/// The write tools and their `(destructiveHint, idempotentHint)`; every
/// other tool is read-only.
const WRITE_TOOLS: &[(&str, bool, bool)] = &[
    ("create_rule_set", false, false),
    ("update_rule_set", true, true),
    ("set_rule_set_enabled", true, true),
    ("upsert_delay_profile", true, true),
    ("save_quality_profile_settings", true, true),
    ("update_indexer_routing", true, true),
    ("update_download_client_routing", true, true),
    ("reorder_download_clients", true, true),
    ("set_indexer_download_client_mapping", true, true),
];

/// A small valid rule module.
const FIXTURE_REGO: &str = "package scryer.rules.user.fixture\nimport rego.v1\n\n\
score_entry[\"fixture_web\"] := 50 if {\n\tinput.release.source == \"WEB-DL\"\n}\n";

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
        ("list_rule_sets", json!({})),
        ("get_rule_set", json!({"id": "mcp-fixture-missing-rule"})),
        ("list_title_episodes", json!({"title_id": title_id})),
        ("validate_rule_set", json!({"rego_source": FIXTURE_REGO})),
        (
            "test_rule_set",
            json!({
                "title_id": title_id,
                "release_name": "Fixture.Probe.Film.2024.1080p.WEB-DL-GRP",
                "rego_source": FIXTURE_REGO,
            }),
        ),
        ("list_delay_profiles", json!({})),
        ("get_quality_profile_settings", json!({})),
        ("get_indexer_routing", json!({"scope": "MOVIE"})),
        ("get_download_client_routing", json!({"scope": "SERIES"})),
        ("list_seeding_profiles", json!({})),
        ("list_indexer_provider_types", json!({})),
        ("list_download_client_provider_types", json!({})),
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
            let annotations = &tool["annotations"];
            match WRITE_TOOLS
                .iter()
                .find(|(write, _, _)| tool["name"] == *write)
            {
                Some((_, destructive, idempotent)) => {
                    assert_eq!(annotations["readOnlyHint"], false, "{name}");
                    assert_eq!(annotations["destructiveHint"], *destructive, "{name}");
                    assert_eq!(annotations["idempotentHint"], *idempotent, "{name}");
                }
                None => {
                    assert_eq!(annotations["readOnlyHint"], true, "{name}");
                    assert!(annotations.get("destructiveHint").is_none(), "{name}");
                }
            }
            assert_eq!(annotations["openWorldHint"], false, "{name}");
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
    seed_quality_profiles(&fixture).await;
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
    seed_quality_profiles(&fixture).await;
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

/// A tool call's refused argument path, from a protocol error.
async fn refused_argument(fixture: &Fixture, name: &str, arguments: Value) -> String {
    let key = fixture.api_key.clone();
    let (_, _, body) = send(
        &fixture.router,
        rpc_request(Some(&key), None, call_tool(name, arguments)),
    )
    .await;
    assert_eq!(
        protocol_error_reason(&body),
        HTTP_MCP_ARGUMENTS_INVALID,
        "{name}: {body}"
    );
    body["error"]["data"]["argument"]
        .as_str()
        .unwrap_or_default()
        .to_string()
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

fn operation_document(name: &str) -> &'static str {
    tools::every_operation_document()
        .into_iter()
        .find(|(document, _)| *document == name)
        .map(|(_, document)| document)
        .unwrap_or_else(|| panic!("no document {name}"))
}

/// Every write tool with valid arguments and the variables `/graphql` gets
/// for the same call.
fn write_calls(rule_id: &str) -> Vec<(&'static str, Value, Value)> {
    let profile = json!({
        "id": "mcp-fixture-delay",
        "name": "Fixture Delay",
        "usenetDelayMinutes": 0,
        "torrentDelayMinutes": 30,
        "enableUsenet": true,
        "enableTorrent": true,
        "preferredProtocol": "USENET",
        "minAgeMinutes": 0,
        "bypassIfHighestQuality": false,
        "appliesToFacets": ["MOVIE"],
        "tags": [],
        "priority": 1,
        "enabled": true,
    });
    vec![
        (
            "create_rule_set",
            json!({"name": "Fixture Refused", "rego_source": FIXTURE_REGO}),
            json!({"input": {"name": "Fixture Refused", "regoSource": FIXTURE_REGO, "enabled": false}}),
        ),
        (
            "update_rule_set",
            json!({"id": rule_id, "name": "Fixture Refused"}),
            json!({"input": {"id": rule_id, "name": "Fixture Refused"}}),
        ),
        (
            "set_rule_set_enabled",
            json!({"id": rule_id, "enabled": true}),
            json!({"input": {"id": rule_id, "enabled": true}}),
        ),
        (
            "upsert_delay_profile",
            json!({"profile": profile}),
            json!({"input": profile}),
        ),
        (
            "save_quality_profile_settings",
            json!({}),
            json!({"input": {
                "profiles": [],
                "categorySelections": [],
                "categoryPersonaSelections": [],
                "replaceExisting": false,
            }}),
        ),
        (
            "update_indexer_routing",
            json!({"scope": "MOVIE", "entries": []}),
            json!({"input": {"scope": "MOVIE", "entries": []}}),
        ),
        (
            "update_download_client_routing",
            json!({"scope": "MOVIE", "entries": []}),
            json!({"input": {"scope": "MOVIE", "entries": []}}),
        ),
        (
            "reorder_download_clients",
            json!({"ids": ["mcp-fixture-client"]}),
            json!({"input": {"ids": ["mcp-fixture-client"]}}),
        ),
        (
            "set_indexer_download_client_mapping",
            json!({"indexer_id": "mcp-fixture-indexer"}),
            json!({"input": {"indexerId": "mcp-fixture-indexer", "downloadClientId": null}}),
        ),
    ]
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

/// Seeds season 1 with one episode for a series and returns the episode id.
async fn seed_episode(context: &common::TestContext, title_id: &str) -> String {
    use scryer_application::ShowRepository;
    let season = within(
        "create season",
        context.shows.create_collection(scryer_domain::Collection {
            id: Id::new().0,
            title_id: title_id.to_string(),
            collection_type: scryer_domain::CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: None,
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            monitored: true,
            created_at: chrono::Utc::now(),
        }),
    )
    .await
    .expect("create season");
    let episode = within(
        "create episode",
        context.shows.create_episode(scryer_domain::Episode {
            id: Id::new().0,
            title_id: title_id.to_string(),
            collection_id: Some(season.id),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Fixture Pilot".to_string()),
            air_date: None,
            duration_seconds: Some(1440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            contiguous_absolute_number: None,
            overview: None,
            tvdb_id: None,
            tmdb_id: None,
            image_url: None,
            monitored: true,
            created_at: chrono::Utc::now(),
        }),
    )
    .await
    .expect("create episode");
    episode.id
}

/// What the rule and acquisition reads return, read as the administrator.
async fn saved_acquisition_state(fixture: &Fixture) -> Value {
    let key = fixture.api_key.clone();
    let mut state = serde_json::Map::new();
    for (name, arguments) in [
        ("list_rule_sets", json!({})),
        ("list_delay_profiles", json!({})),
        ("get_quality_profile_settings", json!({"limit": 10})),
        ("get_indexer_routing", json!({"scope": "MOVIE"})),
        ("get_download_client_routing", json!({"scope": "MOVIE"})),
        ("list_download_clients", json!({})),
        ("list_indexers", json!({})),
    ] {
        let content = success(name, &call(fixture, &key, name, arguments).await);
        state.insert(name.into(), content);
    }
    Value::Object(state)
}

fn item_with<'v>(content: &'v Value, key: &str, value: &str) -> &'v Value {
    content["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item[key] == value))
        .unwrap_or_else(|| panic!("no item with {key} {value}: {content}"))
}

#[tokio::test]
async fn rule_set_tools_create_update_and_toggle_through_the_reads() {
    let fixture = fixture().await;
    seed_quality_profiles(&fixture).await;
    let key = fixture.api_key.clone();

    let created = success(
        "create_rule_set",
        &call(
            &fixture,
            &key,
            "create_rule_set",
            json!({
                "name": "Fixture Web Bonus",
                "rego_source": FIXTURE_REGO,
                "applied_facets": ["movie"],
                "priority": 5,
            }),
        )
        .await,
    );
    let rule = &created["createRuleSet"];
    let id = rule["id"].as_str().expect("rule id").to_string();
    assert_eq!(rule["enabled"], false, "{created}");

    let read = success(
        "get_rule_set",
        &call(&fixture, &key, "get_rule_set", json!({"id": id})).await,
    );
    let saved = &read["ruleSet"];
    assert_eq!(saved["name"], "Fixture Web Bonus", "{read}");
    assert_eq!(saved["priority"], 5, "{read}");
    assert_eq!(saved["appliedFacets"], json!(["movie"]), "{read}");
    assert_eq!(saved["rego_source_omitted"], false, "{read}");
    assert!(
        saved["regoSource"]
            .as_str()
            .is_some_and(|source| source.contains("fixture_web")),
        "{read}"
    );
    assert_eq!(
        saved["rego_source_chars"],
        saved["regoSource"].as_str().unwrap().chars().count(),
        "{read}"
    );

    success(
        "update_rule_set",
        &call(
            &fixture,
            &key,
            "update_rule_set",
            json!({"id": id, "name": "Fixture Web Bonus Two", "priority": 6}),
        )
        .await,
    );
    let read = success(
        "get_rule_set",
        &call(&fixture, &key, "get_rule_set", json!({"id": id})).await,
    );
    assert_eq!(read["ruleSet"]["name"], "Fixture Web Bonus Two", "{read}");
    assert_eq!(read["ruleSet"]["priority"], 6, "{read}");
    assert!(
        read["ruleSet"]["regoSource"]
            .as_str()
            .is_some_and(|source| source.contains("fixture_web")),
        "an update without a source keeps it: {read}"
    );

    success(
        "set_rule_set_enabled",
        &call(
            &fixture,
            &key,
            "set_rule_set_enabled",
            json!({"id": id, "enabled": true}),
        )
        .await,
    );
    let listed = success(
        "list_rule_sets",
        &call(&fixture, &key, "list_rule_sets", json!({})).await,
    );
    assert_eq!(item_with(&listed, "id", &id)["enabled"], true, "{listed}");
    assert!(listed["enabled"].as_u64().is_some_and(|count| count >= 1));
    let rule_count = listed["total"].clone();

    // The saved rule scores a release.
    let title_id = seed_title_with_decisions(&fixture.context, &[]).await;
    success(
        "test_rule_set",
        &call(
            &fixture,
            &key,
            "test_rule_set",
            json!({
                "title_id": title_id,
                "release_name": "Fixture.Probe.Film.2024.1080p.WEB-DL-GRP",
                "rego_source": FIXTURE_REGO,
            }),
        )
        .await,
    );
    let tested = success(
        "test_rule_set",
        &call(
            &fixture,
            &key,
            "test_rule_set",
            json!({
                "title_id": title_id,
                "release_name": "Fixture.Probe.Film.2024.1080p.WEB-DL-GRP",
                "rule_set_id": id,
            }),
        )
        .await,
    );
    assert!(tested["score"].is_i64(), "{tested}");
    assert!(tested["rule_set_count"].is_u64(), "{tested}");

    // Diagnostics are fields, not prose.
    let checked = success(
        "validate_rule_set",
        &call(
            &fixture,
            &key,
            "validate_rule_set",
            json!({"rego_source": "package scryer.rules.user.fixture\nimport rego.v1\n\nscore_entry[\"x\"] := if {\n"}),
        )
        .await,
    );
    assert_eq!(checked["valid"], false, "{checked}");
    let diagnostic = &checked["diagnostics"][0];
    assert!(diagnostic["code"].is_string(), "{checked}");
    assert!(
        checked["diagnostic_count"]
            .as_u64()
            .is_some_and(|count| count >= 1)
    );
    let checked = success(
        "validate_rule_set",
        &call(
            &fixture,
            &key,
            "validate_rule_set",
            json!({"rego_source": FIXTURE_REGO}),
        )
        .await,
    );
    assert_eq!(checked["valid"], true, "{checked}");

    // The dry runs saved no rule set.
    let listed = success(
        "list_rule_sets",
        &call(&fixture, &key, "list_rule_sets", json!({})).await,
    );
    assert_eq!(listed["total"], rule_count, "{listed}");

    // A missing rule set is an empty answer.
    let missing = success(
        "get_rule_set",
        &call(
            &fixture,
            &key,
            "get_rule_set",
            json!({"id": "mcp-fixture-missing-rule"}),
        )
        .await,
    );
    assert!(missing["ruleSet"].is_null(), "{missing}");
}

#[tokio::test]
async fn a_draft_tested_against_a_saved_rule_keeps_its_facets_and_priority() {
    let fixture = fixture().await;
    seed_quality_profiles(&fixture).await;
    let key = fixture.api_key.clone();
    let created = success(
        "create_rule_set",
        &call(
            &fixture,
            &key,
            "create_rule_set",
            json!({
                "name": "Fixture Movie Only",
                "rego_source": FIXTURE_REGO,
                "applied_facets": ["movie"],
                "priority": 4,
            }),
        )
        .await,
    );
    let rule_id = created["createRuleSet"]["id"]
        .as_str()
        .expect("rule id")
        .to_string();
    let movie_id = seed_title_with_decisions(&fixture.context, &[]).await;
    let mut series = fixture_title("Fixture Probe Show");
    series.facet = MediaFacet::Series;
    series.library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    series.root_folder_id = scryer_domain::root_folder_id_for_path("/data/series");
    let series = within("create series", fixture.context.titles.create(series))
        .await
        .expect("create series");
    let episode_id = seed_episode(&fixture.context, &series.id).await;
    let draft = "package scryer.rules.user.fixture\nimport rego.v1\n\n\
score_entry[\"fixture_any\"] := 40\n";
    let test = |title_id: String, extra: Value| {
        let is_series = title_id == series.id;
        let mut arguments = json!({
            "title_id": title_id,
            "release_name": "Fixture.Probe.2024.1080p.WEB-DL-GRP",
            "rego_source": draft,
        });
        if is_series {
            arguments["episode_id"] = json!(episode_id);
            arguments["release_name"] = json!("Fixture.Probe.Show.S01E01.1080p.WEB-DL-GRP");
        }
        for (name, value) in extra.as_object().cloned().unwrap_or_default() {
            arguments[name] = value;
        }
        arguments
    };

    // With the saved rule, the draft keeps its movie-only facets.
    let tested = success(
        "test_rule_set",
        &call(
            &fixture,
            &key,
            "test_rule_set",
            test(series.id.clone(), json!({"rule_set_id": rule_id})),
        )
        .await,
    );
    let contribution = &tested["draftContribution"];
    assert_eq!(contribution["applies"], false, "{tested}");
    assert_eq!(contribution["score"], 0, "{tested}");
    // The same draft scores a movie, so the source itself contributes.
    let tested = success(
        "test_rule_set",
        &call(
            &fixture,
            &key,
            "test_rule_set",
            test(movie_id, json!({"rule_set_id": rule_id})),
        )
        .await,
    );
    assert_eq!(tested["draftContribution"]["applies"], true, "{tested}");
    assert_eq!(tested["draftContribution"]["score"], 40, "{tested}");
    assert_eq!(tested["draftContribution"]["enabled"], true, "{tested}");
    // Facets given in the call win over the saved ones.
    let tested = success(
        "test_rule_set",
        &call(
            &fixture,
            &key,
            "test_rule_set",
            test(
                series.id.clone(),
                json!({"rule_set_id": rule_id, "applied_facets": ["series"]}),
            ),
        )
        .await,
    );
    assert_eq!(tested["draftContribution"]["applies"], true, "{tested}");
    // Without a saved rule, the draft applies to every facet.
    let tested = success(
        "test_rule_set",
        &call(
            &fixture,
            &key,
            "test_rule_set",
            test(series.id.clone(), json!({})),
        )
        .await,
    );
    assert_eq!(tested["draftContribution"]["applies"], true, "{tested}");
}

#[tokio::test]
async fn get_rule_set_omits_a_source_over_the_cap_and_states_its_length() {
    let fixture = fixture().await;
    seed_quality_profiles(&fixture).await;
    let key = fixture.api_key.clone();
    // Saved through `/graphql`, which takes a longer source than the tool.
    let source = format!("{FIXTURE_REGO}# {}\n", "x".repeat(30_500));
    let created = graphql_body(
        &fixture,
        &key,
        operation_document("create_rule_set"),
        json!({"input": {"name": "Fixture Long Rule", "regoSource": source, "enabled": false}}),
    )
    .await;
    let id = created["data"]["createRuleSet"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("rule id: {created}"))
        .to_string();
    let read = success(
        "get_rule_set",
        &call(&fixture, &key, "get_rule_set", json!({"id": id})).await,
    );
    let saved = &read["ruleSet"];
    assert!(saved["regoSource"].is_null(), "{read}");
    assert_eq!(saved["rego_source_omitted"], true, "{read}");
    // The count is of the source as saved, which `/graphql` returns whole.
    let stored = graphql_body(
        &fixture,
        &key,
        operation_document("get_rule_set"),
        json!({"id": id}),
    )
    .await;
    let stored = stored["data"]["ruleSet"]["regoSource"]
        .as_str()
        .unwrap_or_else(|| panic!("stored source: {stored}"));
    assert!(stored.chars().count() > 30_000);
    assert_eq!(saved["rego_source_chars"], stored.chars().count(), "{read}");
}

#[tokio::test]
async fn quality_profile_fields_left_out_clear_or_keep_as_described() {
    let fixture = fixture().await;
    seed_quality_profiles(&fixture).await;
    let key = fixture.api_key.clone();
    let settings = success(
        "get_quality_profile_settings",
        &call(&fixture, &key, "get_quality_profile_settings", json!({})).await,
    );
    let mut criteria = settings["items"][0]["criteria"].clone();
    let tier = criteria["qualityTiers"][0].clone();
    criteria["archivalQuality"] = tier.clone();
    criteria["cutoffTier"] = tier;
    criteria["minScoreToGrab"] = json!(10);
    criteria["cutoffScore"] = json!(500);
    criteria["minUpgradeScoreDelta"] = json!(7);
    criteria["minMissingEpisodesForPack"] = json!(5);
    criteria["minMissingPercentForSeriesPack"] = json!(60);
    criteria["ignoreProperRepack"] = json!(true);
    criteria["scoringOverrides"]["allowX265Non4K"] = json!(true);
    criteria.as_object_mut().unwrap().remove("decisionPipeline");
    let save = |criteria: &Value| {
        json!({"profiles": [{
            "id": "mcp-fixture-quality",
            "name": "Fixture Quality",
            "criteria": criteria,
        }]})
    };
    let read =
        |settings: &Value| item_with(settings, "id", "mcp-fixture-quality")["criteria"].clone();
    success(
        "save_quality_profile_settings",
        &call(
            &fixture,
            &key,
            "save_quality_profile_settings",
            save(&criteria),
        )
        .await,
    );
    let first = read(&success(
        "get_quality_profile_settings",
        &call(
            &fixture,
            &key,
            "get_quality_profile_settings",
            json!({"limit": 10}),
        )
        .await,
    ));
    assert_eq!(first["cutoffScore"], 500, "{first}");
    assert_eq!(first["minUpgradeScoreDelta"], 7, "{first}");
    assert_eq!(first["ignoreProperRepack"], true, "{first}");

    let mut omitted = criteria.clone();
    let fields = omitted.as_object_mut().unwrap();
    for name in [
        "archivalQuality",
        "cutoffTier",
        "minScoreToGrab",
        "cutoffScore",
        "minUpgradeScoreDelta",
        "minMissingEpisodesForPack",
        "minMissingPercentForSeriesPack",
        "ignoreProperRepack",
    ] {
        fields.remove(name);
    }
    fields["scoringOverrides"]
        .as_object_mut()
        .unwrap()
        .remove("allowX265Non4K");
    success(
        "save_quality_profile_settings",
        &call(
            &fixture,
            &key,
            "save_quality_profile_settings",
            save(&omitted),
        )
        .await,
    );
    let second = read(&success(
        "get_quality_profile_settings",
        &call(
            &fixture,
            &key,
            "get_quality_profile_settings",
            json!({"limit": 10}),
        )
        .await,
    ));
    // Read as sent: left out, these clear.
    for name in [
        "archivalQuality",
        "cutoffTier",
        "minScoreToGrab",
        "cutoffScore",
    ] {
        assert!(second[name].is_null(), "{name} clears: {second}");
    }
    assert_ne!(
        second["scoringOverrides"]["allowX265Non4K"], true,
        "{second}"
    );
    // Left out, these keep the saved value.
    for name in [
        "minUpgradeScoreDelta",
        "minMissingEpisodesForPack",
        "minMissingPercentForSeriesPack",
        "ignoreProperRepack",
        "decisionPipeline",
    ] {
        assert_eq!(second[name], first[name], "{name} keeps: {second}");
    }
}

#[tokio::test]
async fn acquisition_writes_show_through_the_matching_reads() {
    let fixture = fixture().await;
    seed_quality_profiles(&fixture).await;
    let key = fixture.api_key.clone();
    seed_integrations(
        &fixture.context,
        &["mcp-fixture-indexer"],
        &["mcp-fixture-client-a", "mcp-fixture-client-b"],
    )
    .await;

    // Delay profile.
    let (_, profile_args, _) = write_calls("unused")
        .into_iter()
        .find(|(name, _, _)| *name == "upsert_delay_profile")
        .expect("delay profile call");
    success(
        "upsert_delay_profile",
        &call(&fixture, &key, "upsert_delay_profile", profile_args.clone()).await,
    );
    let listed = success(
        "list_delay_profiles",
        &call(&fixture, &key, "list_delay_profiles", json!({})).await,
    );
    let saved = item_with(&listed, "id", "mcp-fixture-delay");
    assert_eq!(saved["name"], "Fixture Delay", "{listed}");
    assert_eq!(saved["torrentDelayMinutes"], 30, "{listed}");
    let mut changed = profile_args;
    changed["profile"]["torrentDelayMinutes"] = json!(45);
    success(
        "upsert_delay_profile",
        &call(&fixture, &key, "upsert_delay_profile", changed).await,
    );
    let listed = success(
        "list_delay_profiles",
        &call(&fixture, &key, "list_delay_profiles", json!({})).await,
    );
    assert_eq!(
        item_with(&listed, "id", "mcp-fixture-delay")["torrentDelayMinutes"],
        45,
        "{listed}"
    );

    // Quality profile: a copy of an existing profile's criteria, round-tripped
    // from the read into the write.
    let settings = success(
        "get_quality_profile_settings",
        &call(&fixture, &key, "get_quality_profile_settings", json!({})).await,
    );
    let template = settings["items"][0].clone();
    assert!(template["criteria"].is_object(), "{settings}");
    let profiles_before = settings["total"].as_u64().expect("total");
    let saved = success(
        "save_quality_profile_settings",
        &call(
            &fixture,
            &key,
            "save_quality_profile_settings",
            json!({"profiles": [{
                "id": "mcp-fixture-quality",
                "name": "Fixture Quality",
                "criteria": template["criteria"],
            }]}),
        )
        .await,
    );
    assert!(saved["saveQualityProfileSettings"].is_object(), "{saved}");
    let settings = success(
        "get_quality_profile_settings",
        &call(
            &fixture,
            &key,
            "get_quality_profile_settings",
            json!({"limit": 10}),
        )
        .await,
    );
    assert_eq!(
        settings["total"],
        profiles_before + 1,
        "a save merges: {settings}"
    );
    let saved = item_with(&settings, "id", "mcp-fixture-quality");
    assert_eq!(saved["name"], "Fixture Quality", "{settings}");
    assert_eq!(saved["criteria"], template["criteria"], "{settings}");

    // Indexer routing.
    success(
        "update_indexer_routing",
        &call(
            &fixture,
            &key,
            "update_indexer_routing",
            json!({"scope": "MOVIE", "entries": [{
                "indexerId": "mcp-fixture-indexer",
                "enabled": true,
                "categories": ["2000"],
                "priority": 7,
            }]}),
        )
        .await,
    );
    let routing = success(
        "get_indexer_routing",
        &call(
            &fixture,
            &key,
            "get_indexer_routing",
            json!({"scope": "MOVIE"}),
        )
        .await,
    );
    let entry = item_with(&routing, "indexerId", "mcp-fixture-indexer");
    assert_eq!(entry["priority"], 7, "{routing}");
    assert_eq!(entry["categories"], json!(["2000"]), "{routing}");

    // What the read returns goes back through the write unchanged, with
    // repeated categories and a priority at the edge of the `Int` range.
    let mut entry = entry.clone();
    entry["categories"] = json!(["2000", "2000"]);
    entry["priority"] = json!(i32::MIN);
    success(
        "update_indexer_routing",
        &call(
            &fixture,
            &key,
            "update_indexer_routing",
            json!({"scope": "MOVIE", "entries": [entry]}),
        )
        .await,
    );
    let routing = success(
        "get_indexer_routing",
        &call(
            &fixture,
            &key,
            "get_indexer_routing",
            json!({"scope": "MOVIE"}),
        )
        .await,
    );
    let read_back = item_with(&routing, "indexerId", "mcp-fixture-indexer").clone();
    assert_eq!(read_back["priority"], i32::MIN, "{routing}");
    assert_eq!(
        read_back["categories"],
        json!(["2000", "2000"]),
        "{routing}"
    );
    success(
        "update_indexer_routing",
        &call(
            &fixture,
            &key,
            "update_indexer_routing",
            json!({"scope": "MOVIE", "entries": [read_back]}),
        )
        .await,
    );

    // Download-client routing.
    success(
        "update_download_client_routing",
        &call(
            &fixture,
            &key,
            "update_download_client_routing",
            json!({"scope": "SERIES", "entries": [{
                "clientId": "mcp-fixture-client-a",
                "enabled": true,
                "category": "fixture-tv",
                "removeCompleted": false,
                "removeFailed": true,
            }]}),
        )
        .await,
    );
    let routing = success(
        "get_download_client_routing",
        &call(
            &fixture,
            &key,
            "get_download_client_routing",
            json!({"scope": "SERIES"}),
        )
        .await,
    );
    let entry = item_with(&routing, "clientId", "mcp-fixture-client-a");
    assert_eq!(entry["category"], "fixture-tv", "{routing}");
    assert_eq!(entry["removeFailed"], true, "{routing}");

    // Client order.
    success(
        "reorder_download_clients",
        &call(
            &fixture,
            &key,
            "reorder_download_clients",
            json!({"ids": ["mcp-fixture-client-b", "mcp-fixture-client-a"]}),
        )
        .await,
    );
    let clients = success(
        "list_download_clients",
        &call(&fixture, &key, "list_download_clients", json!({})).await,
    );
    let order = clients["items"]
        .as_array()
        .expect("clients")
        .iter()
        .filter_map(|client| client["id"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        ["mcp-fixture-client-b", "mcp-fixture-client-a"],
        "{clients}"
    );

    // Indexer to client pin, then cleared.
    success(
        "set_indexer_download_client_mapping",
        &call(
            &fixture,
            &key,
            "set_indexer_download_client_mapping",
            json!({"indexer_id": "mcp-fixture-indexer", "download_client_id": "mcp-fixture-client-b"}),
        )
        .await,
    );
    let indexers = success(
        "list_indexers",
        &call(&fixture, &key, "list_indexers", json!({})).await,
    );
    assert_eq!(
        item_with(&indexers, "id", "mcp-fixture-indexer")["downloadClientId"],
        "mcp-fixture-client-b",
        "{indexers}"
    );
    success(
        "set_indexer_download_client_mapping",
        &call(
            &fixture,
            &key,
            "set_indexer_download_client_mapping",
            json!({"indexer_id": "mcp-fixture-indexer"}),
        )
        .await,
    );
    let indexers = success(
        "list_indexers",
        &call(&fixture, &key, "list_indexers", json!({})).await,
    );
    assert!(
        item_with(&indexers, "id", "mcp-fixture-indexer")["downloadClientId"].is_null(),
        "{indexers}"
    );
}

#[tokio::test]
async fn write_tools_refuse_a_low_permission_key_as_graphql_does() {
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
    seed_quality_profiles(&fixture).await;
    seed_integrations(
        &fixture.context,
        &["mcp-fixture-indexer"],
        &[
            "mcp-fixture-client-y",
            "mcp-fixture-client-z",
            "mcp-fixture-client",
        ],
    )
    .await;
    // Saved state each refused write would change.
    let admin_key = fixture.api_key.clone();
    for (name, arguments) in [
        (
            "update_indexer_routing",
            json!({"scope": "MOVIE", "entries": [{
                "indexerId": "mcp-fixture-indexer",
                "enabled": true,
                "categories": ["2000"],
                "priority": 3,
            }]}),
        ),
        (
            "update_download_client_routing",
            json!({"scope": "MOVIE", "entries": [{
                "clientId": "mcp-fixture-client",
                "enabled": true,
                "category": "fixture-movies",
                "removeCompleted": false,
                "removeFailed": false,
            }]}),
        ),
        (
            "set_indexer_download_client_mapping",
            json!({"indexer_id": "mcp-fixture-indexer", "download_client_id": "mcp-fixture-client"}),
        ),
    ] {
        success(name, &call(&fixture, &admin_key, name, arguments).await);
    }
    let before = saved_acquisition_state(&fixture).await;

    for (name, arguments, variables) in write_calls("mcp-fixture-rule") {
        let body = call(&fixture, &limited_key, name, arguments).await;
        let error = tool_error(name, &body);
        let expected =
            graphql_refusal(&fixture, &limited_key, operation_document(name), variables).await;
        assert_eq!(error["code"], expected["code"], "{name}: {body}");
        assert_eq!(error["code"], "UNAUTHORIZED", "{name}: {body}");
        assert_eq!(error["reason"], expected["reason"], "{name}: {body}");
    }
    for name in ["validate_rule_set", "test_rule_set"] {
        let (_, arguments) = minimal_calls("mcp-fixture-title")
            .into_iter()
            .find(|(call, _)| *call == name)
            .expect("dry-run call");
        let body = call(&fixture, &limited_key, name, arguments).await;
        assert_eq!(tool_error(name, &body)["code"], "UNAUTHORIZED", "{body}");
    }
    // Nothing was saved: rules, delay and quality profiles, both routings,
    // the client order and the indexer's client pin read as before.
    let after = saved_acquisition_state(&fixture).await;
    assert_eq!(after, before);
    let order = after["list_download_clients"]["items"]
        .as_array()
        .expect("clients")
        .iter()
        .filter_map(|client| client["id"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        [
            "mcp-fixture-client-y",
            "mcp-fixture-client-z",
            "mcp-fixture-client"
        ]
    );
    assert_eq!(
        item_with(&after["list_indexers"], "id", "mcp-fixture-indexer")["downloadClientId"],
        "mcp-fixture-client"
    );
    assert_eq!(
        item_with(
            &after["get_indexer_routing"],
            "indexerId",
            "mcp-fixture-indexer"
        )["priority"],
        3
    );
    assert_eq!(
        item_with(
            &after["get_download_client_routing"],
            "clientId",
            "mcp-fixture-client"
        )["category"],
        "fixture-movies"
    );
    assert!(
        after["list_delay_profiles"]["items"]
            .as_array()
            .is_some_and(|items| items.iter().all(|item| item["id"] != "mcp-fixture-delay")),
        "{after}"
    );
    assert!(
        after["list_rule_sets"]["items"]
            .as_array()
            .is_some_and(|items| items.iter().all(|item| item["name"] != "Fixture Refused")),
        "{after}"
    );
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

    // The owner has no TOTP credential, so every configuration write and
    // dry-run is refused for enrollment, exactly as `/graphql` refuses it.
    for (name, arguments, variables) in write_calls("mcp-fixture-rule") {
        let body = call(&fixture, &key, name, arguments).await;
        let error = tool_error(name, &body);
        let expected = graphql_refusal(&fixture, &key, operation_document(name), variables).await;
        assert_eq!(error["code"], "TOTP_ENROLLMENT_REQUIRED", "{name}: {body}");
        assert_eq!(error["code"], expected["code"], "{name}: {body}");
        assert_eq!(error["reason"], expected["reason"], "{name}: {body}");
    }
    let title_id = seed_title_with_decisions(&fixture.context, &[]).await;
    for name in ["validate_rule_set", "test_rule_set"] {
        let (_, arguments) = minimal_calls(&title_id)
            .into_iter()
            .find(|(call, _)| *call == name)
            .expect("dry-run call");
        let body = call(&fixture, &key, name, arguments).await;
        assert_eq!(
            tool_error(name, &body)["code"],
            "TOTP_ENROLLMENT_REQUIRED",
            "{body}"
        );
    }
    // Reads are not configuration changes.
    for name in [
        "list_rule_sets",
        "list_delay_profiles",
        "get_quality_profile_settings",
    ] {
        success(name, &call(&fixture, &key, name, json!({})).await);
    }
}

#[tokio::test]
async fn write_tool_arguments_are_checked_field_by_field() {
    let fixture = fixture().await;
    let (_, profile_args, _) = write_calls("unused")
        .into_iter()
        .find(|(name, _, _)| *name == "upsert_delay_profile")
        .expect("delay profile call");

    let mut missing = profile_args.clone();
    missing["profile"].as_object_mut().unwrap().remove("name");
    assert_eq!(
        refused_argument(&fixture, "upsert_delay_profile", missing).await,
        "profile.name"
    );
    let mut extra = profile_args.clone();
    extra["profile"]["mcpfixtureextra"] = json!(1);
    assert_eq!(
        refused_argument(&fixture, "upsert_delay_profile", extra).await,
        "profile"
    );
    let mut protocol = profile_args.clone();
    protocol["profile"]["preferredProtocol"] = json!("CARRIER_PIGEON");
    assert_eq!(
        refused_argument(&fixture, "upsert_delay_profile", protocol).await,
        "profile.preferredProtocol"
    );
    let entry = json!({"indexerId": "i", "enabled": true, "categories": [], "priority": 1});
    let mut out_of_range = entry.clone();
    out_of_range["priority"] = json!(i64::from(i32::MAX) + 1);
    assert_eq!(
        refused_argument(
            &fixture,
            "update_indexer_routing",
            json!({"scope": "MOVIE", "entries": [entry, out_of_range]}),
        )
        .await,
        "entries[1].priority"
    );
    assert_eq!(
        refused_argument(
            &fixture,
            "update_download_client_routing",
            json!({"scope": "MOVIE", "entries": [{"clientId": "c", "enabled": true}]}),
        )
        .await,
        "entries[0].removeCompleted"
    );
    assert_eq!(
        refused_argument(
            &fixture,
            "save_quality_profile_settings",
            json!({"profiles": [{"id": "q", "name": "Q", "criteria": {}}]}),
        )
        .await,
        "profiles[0].criteria.qualityTiers"
    );
    assert_eq!(
        refused_argument(&fixture, "reorder_download_clients", json!({"ids": []})).await,
        "ids"
    );
    assert_eq!(
        refused_argument(&fixture, "set_rule_set_enabled", json!({"id": "r"})).await,
        "enabled"
    );
    assert_eq!(
        refused_argument(&fixture, "update_rule_set", json!({"id": "r"})).await,
        "rego_source"
    );
    assert_eq!(
        refused_argument(
            &fixture,
            "create_rule_set",
            json!({"name": "Fixture", "rego_source": FIXTURE_REGO, "applied_facets": ["tv"]}),
        )
        .await,
        "applied_facets"
    );
    assert_eq!(
        refused_argument(
            &fixture,
            "create_rule_set",
            json!({"name": "Fixture", "rego_source": "x".repeat(30_001)}),
        )
        .await,
        "rego_source"
    );
    assert_eq!(
        refused_argument(
            &fixture,
            "test_rule_set",
            json!({"title_id": "t", "release_name": "Fixture.Release"}),
        )
        .await,
        "rego_source"
    );
    assert_eq!(
        refused_argument(
            &fixture,
            "list_title_episodes",
            json!({"title_id": "t", "collection_id": "c"}),
        )
        .await,
        "collection_id"
    );
    assert_eq!(
        refused_argument(&fixture, "get_indexer_routing", json!({"scope": "TV"})).await,
        "scope"
    );
}

#[tokio::test]
async fn list_tools_page_within_the_items_budget() {
    let fixture = fixture().await;
    let key = fixture.api_key.clone();
    let (_, profile_args, _) = write_calls("unused")
        .into_iter()
        .find(|(name, _, _)| *name == "upsert_delay_profile")
        .expect("delay profile call");
    let count = 60;
    for index in 0..count {
        let mut arguments = profile_args.clone();
        arguments["profile"]["id"] = json!(format!("mcp-fixture-delay-{index:02}"));
        arguments["profile"]["name"] = json!(format!("{index:02} {}", "Fixture Delay ".repeat(14)));
        arguments["profile"]["tags"] = json!(
            (0..20)
                .map(|tag| format!("fixture-tag-{tag}"))
                .collect::<Vec<_>>()
        );
        success(
            "upsert_delay_profile",
            &call(&fixture, &key, "upsert_delay_profile", arguments).await,
        );
    }
    let mut seen = 0;
    let mut offset = 0;
    loop {
        let page = success(
            "list_delay_profiles",
            &call(
                &fixture,
                &key,
                "list_delay_profiles",
                json!({"offset": offset}),
            )
            .await,
        );
        assert_eq!(page["total"], count, "{page}");
        let returned = page["returned"].as_u64().expect("returned");
        assert!(
            returned >= 1 && returned < 50,
            "the budget cuts the page: {returned}"
        );
        seen += returned;
        match page["next_offset"].as_u64() {
            Some(next) => offset = next,
            None => break,
        }
    }
    assert_eq!(seen, count);
}

#[test]
fn every_new_tool_document_is_covered_by_the_contract_list() {
    // Tools whose documents carry another name, and tools that run no
    // document of their own; every other tool's document is named after it.
    const NAMED_OTHERWISE: &[(&str, &[&str])] = &[
        ("get_health_checks", &["health_checks"]),
        ("list_download_clients", &["download_clients"]),
        ("list_indexers", &["indexers", "indexer_stats"]),
        ("list_indexer_errors", &["indexer_errors"]),
        ("get_system_health", &["system_health"]),
        ("list_job_runs", &["latest_job_runs", "job_runs"]),
        ("get_title_decisions", &["title_decisions"]),
        ("list_pending_releases", &["pending_releases"]),
        ("get_title_history", &["title_history"]),
        ("list_download_queue", &[]),
        ("list_import_history", &["import_history"]),
        ("get_upgrade_status", &["upgrade_status"]),
        ("get_diagnostics_overview", &[]),
        ("read_diagnostics_section", &[]),
        ("search_service_logs", &[]),
        (
            "list_title_episodes",
            &["title_collections", "collection_episodes"],
        ),
    ];
    let documents = tools::every_operation_document();
    let has = |wanted: &str| documents.iter().any(|(document, _)| *document == wanted);
    for name in TOOL_NAMES {
        match NAMED_OTHERWISE.iter().find(|(tool, _)| tool == name) {
            Some((_, named)) => {
                for document in *named {
                    assert!(has(document), "{name}: no document {document}");
                }
            }
            None => assert!(has(name), "{name} has no document in the contract list"),
        }
    }
    for (tool, _) in NAMED_OTHERWISE {
        assert!(TOOL_NAMES.contains(tool), "{tool} is not a tool");
    }
}
