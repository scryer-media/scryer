//! The built-in MCP endpoint: Streamable HTTP, JSON responses only, no
//! server-side sessions.
//!
//! Every request is admitted here before rmcp sees it: the browser Origin must
//! be one the GraphQL WebSocket would accept, and the caller must present an
//! API key. Tools then run ordinary GraphQL operations as that key's actor, so
//! permissions, step-up and library grants match `/graphql` exactly.

mod server;

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

use crate::error_reasons::{
    HTTP_AUTHORIZATION_HEADER_INVALID, HTTP_MCP_API_KEY_INVALID, HTTP_MCP_API_KEY_REQUIRED,
    HTTP_MCP_ORIGIN_NOT_ALLOWED,
};
use crate::middleware::{
    ApiKeyCallerRefusal, AuthState, admit_api_key_caller, restore_maintenance_response,
};

use server::ScryerMcpServer;

/// The route path, relative to the configured base path.
pub(crate) const MCP_PATH: &str = "/mcp";

/// MCP requests are small JSON-RPC envelopes; this stays well under the
/// GraphQL message cap.
const MCP_MAX_REQUEST_BODY_BYTES: usize = 1024 * 1024;

const _: () = assert!(MCP_MAX_REQUEST_BODY_BYTES <= crate::middleware::GRAPHQL_MAX_MESSAGE_BYTES);

#[derive(Clone)]
pub(crate) struct McpState {
    auth: AuthState,
    service: Arc<StreamableHttpService<ScryerMcpServer, NeverSessionManager>>,
}

impl McpState {
    pub(crate) fn new(auth: AuthState) -> Self {
        let factory_auth = auth.clone();
        // Stateless mode answers every request, `initialize` included, on its
        // own; nothing is kept between requests. Host checks are off because
        // LAN and reverse-proxy deployments reach Scryer under any name; the
        // Origin check below covers DNS rebinding.
        let config = StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true)
            .with_sse_keep_alive(None)
            .disable_allowed_hosts()
            .with_max_request_body_bytes(MCP_MAX_REQUEST_BODY_BYTES);
        let service = StreamableHttpService::new(
            move || Ok(ScryerMcpServer::new(factory_auth.clone())),
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
    mut request: Request<Body>,
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
    // Held until the response is built, so a restore waits for this request.
    let _restore_admission = match state.auth.app.restore_coordinator().admit_request().await {
        Ok(guard) => guard,
        Err(_) => return restore_maintenance_response(),
    };

    let caller = match admit_api_key_caller(&state.auth, request.headers(), remote_addr).await {
        Ok(caller) => caller,
        Err(refusal_kind) => return api_key_refusal(refusal_kind),
    };
    request.extensions_mut().insert(caller);

    let response = state.service.handle(request).await;
    let mut response = response.map(Body::new);
    // Tool results belong to one API key's account.
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

fn api_key_refusal(kind: ApiKeyCallerRefusal) -> Response {
    let (reason, message) = match kind {
        ApiKeyCallerRefusal::HeaderInvalid => (
            HTTP_AUTHORIZATION_HEADER_INVALID,
            "The Authorization header must be a single bearer token.",
        ),
        ApiKeyCallerRefusal::Required => (
            HTTP_MCP_API_KEY_REQUIRED,
            "The Scryer MCP endpoint accepts only an API key as a bearer token.",
        ),
        ApiKeyCallerRefusal::Invalid => (
            HTTP_MCP_API_KEY_INVALID,
            "The API key is invalid or no longer authorized.",
        ),
        ApiKeyCallerRefusal::Internal => {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let mut response = refusal(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", reason, message);
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        header::HeaderValue::from_static("Bearer"),
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
