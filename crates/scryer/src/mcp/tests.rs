use super::*;
use crate::middleware::integration_test_common as common;
use crate::middleware::{AuthlessWebClientProofState, WebSocketOriginPolicy};
use crate::rate_limit::ScryerRateLimiter;

use axum::Router;
use axum::http::HeaderValue;
use axum::routing::post;
use scryer_interface::context::{AuthRuntimeStateHandle, AuthRuntimeStateSnapshot};
use serde_json::{Value, json};
use std::net::Ipv4Addr;
use tower::ServiceExt as _;

const HOST: &str = "scryer.lan:8080";

struct Fixture {
    _context: common::TestContext,
    router: Router,
    api_key: String,
    session_token: String,
}

fn auth_state(context: &common::TestContext, auth_runtime: AuthRuntimeStateHandle) -> AuthState {
    AuthState {
        app: context.app.clone(),
        schema: context.schema.clone(),
        auth_runtime,
        rate_limiter: ScryerRateLimiter::from_env(Default::default()),
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

async fn fixture_with(auth_runtime: Option<AuthRuntimeStateHandle>) -> Fixture {
    let context = common::TestContext::new().await;
    let admin = context
        .app
        .find_or_create_default_user()
        .await
        .expect("default administrator");
    let api_key = context
        .app
        .create_api_key(
            &admin,
            scryer_application::CreateApiKey {
                label: "mcp-fixture-key".into(),
                expiry: scryer_application::ApiKeyExpiryPreset::Never,
            },
        )
        .await
        .expect("create API key")
        .raw_key;
    let session_token = context
        .app
        .issue_access_token(&admin)
        .await
        .expect("issue session token");
    let auth_runtime = auth_runtime.unwrap_or_else(|| context.auth_runtime.clone());
    let state = McpState::new(auth_state(&context, auth_runtime));
    let router = Router::new()
        .route(MCP_PATH, post(mcp_handler))
        .with_state(state);
    Fixture {
        _context: context,
        router,
        api_key,
        session_token,
    }
}

fn rpc_request(bearer: Option<&str>, origin: Option<&str>, body: Value) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri(MCP_PATH)
        .header(header::HOST, HOST)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-11-25")
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

async fn send(
    router: &Router,
    request: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let response = router.clone().oneshot(request).await.expect("MCP response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
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

#[tokio::test]
async fn mcp_refuses_every_caller_that_is_not_an_api_key() {
    // The shared fixture leaves authless local access on, so a request with no
    // credentials would resolve the default administrator on `/graphql`.
    let fixture = fixture_with(None).await;

    let (status, headers, body) =
        send(&fixture.router, rpc_request(None, None, list_tools())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["reason"], HTTP_MCP_API_KEY_REQUIRED);
    assert_eq!(body["code"], "UNAUTHORIZED");
    assert_eq!(headers[header::WWW_AUTHENTICATE], "Bearer");

    let session = fixture.session_token.clone();
    let (status, _, body) = send(
        &fixture.router,
        rpc_request(Some(&session), None, list_tools()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["reason"], HTTP_MCP_API_KEY_REQUIRED);

    let prefix = fixture
        .api_key
        .split_once('_')
        .map(|(prefix, _)| prefix)
        .expect("API keys carry a prefix");
    let unknown_key = format!("{prefix}_mcpfixtureunknownkey");
    let (status, _, body) = send(
        &fixture.router,
        rpc_request(Some(&unknown_key), None, list_tools()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["reason"], HTTP_MCP_API_KEY_INVALID);

    let mut malformed = rpc_request(None, None, list_tools());
    malformed.headers_mut().insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("Basic abc def"),
    );
    let (status, _, body) = send(&fixture.router, malformed).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["reason"], HTTP_AUTHORIZATION_HEADER_INVALID);
}

#[tokio::test]
async fn mcp_refuses_a_foreign_origin_and_serves_a_missing_or_same_origin() {
    let fixture = fixture_with(Some(form_login_runtime())).await;
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
async fn mcp_serves_whoami_to_an_api_key() {
    let fixture = fixture_with(Some(form_login_runtime())).await;
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
            .is_some_and(|text| text.len() < 600)
    );

    let (status, _, body) =
        send(&fixture.router, rpc_request(Some(&key), None, list_tools())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let tools = body["result"]["tools"].as_array().expect("tools");
    assert_eq!(tools.len(), 1, "{body}");
    let whoami = &tools[0];
    assert_eq!(whoami["name"], "whoami");
    assert_eq!(whoami["title"], "Who am I");
    assert_eq!(
        whoami["inputSchema"],
        json!({"type": "object", "additionalProperties": false})
    );
    assert_eq!(whoami["annotations"]["readOnlyHint"], true);
    assert_eq!(whoami["annotations"]["openWorldHint"], false);

    let (status, _, body) = send(
        &fixture.router,
        rpc_request(Some(&key), None, call_tool("whoami", json!({}))),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result = &body["result"];
    assert_eq!(result["isError"], false, "{body}");
    let me = &result["structuredContent"]["me"];
    assert_eq!(me["username"], "admin", "{body}");
    assert!(me["appPermissions"].is_array(), "{body}");
    let text = result["content"][0]["text"].as_str().expect("text block");
    assert_eq!(
        serde_json::from_str::<Value>(text).expect("text block is JSON"),
        result["structuredContent"]
    );

    // Protocol faults stay JSON-RPC errors rather than tool results.
    let (_, _, body) = send(
        &fixture.router,
        rpc_request(Some(&key), None, call_tool("not-a-tool", json!({}))),
    )
    .await;
    assert!(body["error"]["code"].is_i64(), "{body}");
    assert!(body.get("result").is_none(), "{body}");
    let (_, _, body) = send(
        &fixture.router,
        rpc_request(
            Some(&key),
            None,
            call_tool("whoami", json!({"user": "someone"})),
        ),
    )
    .await;
    assert!(body["error"]["code"].is_i64(), "{body}");
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
            {"code": "GRAPHQL_ERROR", "message": "Unknown field \"nope\""}
        ]})
    );
}
