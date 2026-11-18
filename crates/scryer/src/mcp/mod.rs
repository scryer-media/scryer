//! The built-in MCP endpoint: Streamable HTTP, JSON responses only, no
//! server-side sessions.
//!
//! Every request is admitted here before rmcp sees it: the browser Origin must
//! be one the GraphQL WebSocket would accept, and the caller must present an
//! API key. Tools then run ordinary GraphQL operations, or application reads
//! that check the same permissions, as that key's actor, so permissions,
//! step-up and library grants match `/graphql` exactly.
//!
//! rmcp's Host allow-list is off, because LAN and reverse-proxy deployments
//! reach Scryer under any name. The Origin check does not stop DNS
//! rebinding: a rebound page sends an Origin equal to the Host it reached,
//! and that passes. What stops it is the API key: a browser page cannot
//! attach a bearer token it does not hold. A Host allow-list must come back
//! before any caller other than an API key is admitted here.

mod server;
mod tools;

#[cfg(test)]
mod operation_contract;
#[cfg(test)]
mod tests;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::Json;
use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::http::{Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};

use crate::diagnostics_routes::DiagnosticsHost;
use crate::error_reasons::{
    HTTP_AUTHORIZATION_HEADER_INVALID, HTTP_MCP_API_KEY_INVALID, HTTP_MCP_API_KEY_REQUIRED,
    HTTP_MCP_ORIGIN_NOT_ALLOWED, HTTP_MCP_REQUEST_BODY_UNREADABLE, HTTP_MCP_REQUEST_TOO_LARGE,
};
use crate::middleware::{
    ApiKeyCallerRefusal, AuthState, admit_api_key_caller, restore_maintenance_response,
};

use server::ScryerMcpServer;

/// The route path, relative to the configured base path.
pub(crate) const MCP_PATH: &str = "/mcp";

/// MCP requests are small JSON-RPC envelopes; this stays well under the
/// GraphQL message cap.
pub(crate) const MCP_MAX_REQUEST_BODY_BYTES: usize = 1024 * 1024;

const _: () = assert!(MCP_MAX_REQUEST_BODY_BYTES <= crate::middleware::GRAPHQL_MAX_MESSAGE_BYTES);

/// The request's hold on the restore coordinator. It travels with the
/// request into rmcp, so it is released only when rmcp drops the request,
/// after the tool's work is gone, and never while that work still runs.
#[derive(Clone)]
pub(crate) struct RestoreLease(#[allow(dead_code)] Arc<tokio::sync::OwnedRwLockReadGuard<()>>);

#[derive(Clone)]
pub(crate) struct McpState {
    auth: AuthState,
    service: Arc<StreamableHttpService<ScryerMcpServer, NeverSessionManager>>,
}

impl McpState {
    pub(crate) fn new(auth: AuthState, diagnostics: Arc<DiagnosticsHost>) -> Self {
        let factory_auth = auth.clone();
        // Stateless mode answers every request, `initialize` included, on its
        // own; nothing is kept between requests. Host checks are off; see the
        // module notes for why that is safe only while every caller is an
        // API key.
        let config = StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true)
            .with_sse_keep_alive(None)
            .disable_allowed_hosts()
            .with_max_request_body_bytes(MCP_MAX_REQUEST_BODY_BYTES);
        let service = StreamableHttpService::new(
            move || {
                Ok(ScryerMcpServer::new(
                    factory_auth.clone(),
                    diagnostics.clone(),
                ))
            },
            Arc::new(NeverSessionManager::default()),
            config,
        );
        Self {
            auth,
            service: Arc::new(service),
        }
    }
}

pub(crate) async fn mcp_handler(
    State(state): State<McpState>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    setup_admission: Option<axum::Extension<crate::initial_auth::SetupAdmission>>,
    request: Request<Body>,
) -> Response {
    let headers = request.headers();
    if let Err(error) = state.auth.ws_origin_policy.check(headers) {
        if state
            .auth
            .ws_origin_policy
            .rejection_warning_due(headers, Instant::now())
        {
            tracing::warn!(
                remote_addr = %remote_addr,
                error = %error,
                "rejecting MCP request because browser Origin is not allowed; repeats from this Origin log at debug for 10 minutes"
            );
        } else {
            tracing::debug!(
                remote_addr = %remote_addr,
                error = %error,
                "rejecting MCP request because browser Origin is not allowed"
            );
        }
        return refusal(
            StatusCode::FORBIDDEN,
            "FORBIDDEN",
            HTTP_MCP_ORIGIN_NOT_ALLOWED,
            "This Origin may not call the Scryer MCP endpoint.",
        );
    }

    let setup = match setup_admission {
        Some(axum::Extension(admission)) => Ok(admission.0),
        None => crate::initial_auth::setup_state(&state.auth).await,
    };
    match setup {
        Ok(setup) if setup.as_ref().is_some_and(|setup| setup.is_pending()) => {
            return crate::initial_auth::refused(
                scryer_application::initial_auth::refusal::REQUIRED,
            );
        }
        Ok(_) => {}
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
    // The whole body is read before the restore lease is taken, so a slow
    // or oversized upload never holds a restore back.
    let (parts, body) = request.into_parts();
    let bytes = match read_body(body, MCP_MAX_REQUEST_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(BodyRefusal::TooLarge) => {
            return refusal(
                StatusCode::PAYLOAD_TOO_LARGE,
                "PAYLOAD_TOO_LARGE",
                HTTP_MCP_REQUEST_TOO_LARGE,
                "The MCP request body is too large.",
            );
        }
        Err(BodyRefusal::Unreadable) => {
            return refusal(
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                HTTP_MCP_REQUEST_BODY_UNREADABLE,
                "The MCP request body could not be read.",
            );
        }
    };
    let mut request = Request::from_parts(parts, Body::from(bytes));

    let lease = match state.auth.app.restore_coordinator().admit_request().await {
        Ok(guard) => RestoreLease(Arc::new(guard)),
        Err(_) => return restore_maintenance_response(),
    };

    let caller = match admit_api_key_caller(&state.auth, request.headers(), remote_addr).await {
        Ok(caller) => caller,
        Err(refusal_kind) => return api_key_refusal(refusal_kind),
    };
    request.extensions_mut().insert(caller);
    request.extensions_mut().insert(lease);

    let response = state.service.handle(request).await;
    let mut response = response.map(Body::new);
    // Tool results belong to one API key's account.
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

enum BodyRefusal {
    TooLarge,
    Unreadable,
}

/// Reads at most `limit` bytes of `body`, stopping at the first frame that
/// passes it, so an oversized body is never buffered whole.
async fn read_body(body: Body, limit: usize) -> Result<axum::body::Bytes, BodyRefusal> {
    use futures_util::StreamExt as _;
    let mut stream = body.into_data_stream();
    let mut buffer = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| BodyRefusal::Unreadable)?;
        if buffer.len() + chunk.len() > limit {
            return Err(BodyRefusal::TooLarge);
        }
        buffer.extend_from_slice(&chunk);
    }
    Ok(buffer.into())
}

fn api_key_refusal(kind: ApiKeyCallerRefusal) -> Response {
    // RFC 6750: a bare challenge when no credentials were offered, an error
    // code when the offered ones are malformed or not accepted.
    let (reason, message, challenge) = match kind {
        ApiKeyCallerRefusal::HeaderInvalid => (
            HTTP_AUTHORIZATION_HEADER_INVALID,
            "The Authorization header must be a single bearer token.",
            "Bearer error=\"invalid_request\"",
        ),
        ApiKeyCallerRefusal::Required => (
            HTTP_MCP_API_KEY_REQUIRED,
            "The Scryer MCP endpoint accepts only an API key as a bearer token.",
            "Bearer",
        ),
        ApiKeyCallerRefusal::NotApiKey => (
            HTTP_MCP_API_KEY_REQUIRED,
            "The Scryer MCP endpoint accepts only an API key as a bearer token.",
            "Bearer error=\"invalid_token\"",
        ),
        ApiKeyCallerRefusal::Invalid => (
            HTTP_MCP_API_KEY_INVALID,
            "The API key is invalid or no longer authorized.",
            "Bearer error=\"invalid_token\"",
        ),
        ApiKeyCallerRefusal::Internal => {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let mut response = refusal(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", reason, message);
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        header::HeaderValue::from_static(challenge),
    );
    response
}

fn refusal(status: StatusCode, code: &str, reason: &str, message: &str) -> Response {
    let mut response = (
        status,
        Json(serde_json::json!({
            "error": message,
            "code": code,
            "reason": reason,
        })),
    )
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}
