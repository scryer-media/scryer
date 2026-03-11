use async_graphql::http::{GraphiQLSource, ALL_WEBSOCKET_PROTOCOLS};
use async_graphql::Data;
use async_graphql_axum::{GraphQLProtocol, GraphQLWebSocket};
use axum::body::Body;
use axum::extract::{State, WebSocketUpgrade};
use axum::http::{header, HeaderMap, Method, Request, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use scryer_application::{AppError, AppUseCase};
use scryer_domain::Entitlement;

use crate::admin_routes::ErrorResponse;
use crate::base_path::BasePath;

#[derive(Clone, Debug)]
pub(crate) struct CorsConfig {
    pub(crate) allow_all: bool,
    pub(crate) allowed_origins: Vec<String>,
}

impl CorsConfig {
    pub(crate) fn from_env() -> Self {
        let raw = std::env::var("SCRYER_CORS_ALLOWED_ORIGINS")
            .unwrap_or_else(|_| default_cors_allowed_origins().join(","));

        let origins = raw
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        let allow_all = origins
            .iter()
            .any(|origin| matches!(origin.as_str(), "*" | "https://*" | "http://*"));

        Self {
            allow_all,
            allowed_origins: origins,
        }
    }

    fn is_allowed(&self, origin: &str) -> bool {
        if self.allow_all {
            return true;
        }
        self.allowed_origins.iter().any(|allowed| allowed == origin)
    }
}

fn default_cors_allowed_origins() -> Vec<String> {
    let mut origins = vec![
        "http://localhost:3000".to_string(),
        "http://127.0.0.1:3000".to_string(),
        "http://0.0.0.0:3000".to_string(),
        "http://host.docker.internal:3000".to_string(),
        "http://nodejs:3000".to_string(),
    ];

    if let Ok(web_ui_url) = std::env::var("SCRYER_WEB_UI_URL") {
        if let Some(web_ui_origin) = canonical_origin(&web_ui_url) {
            push_origin_if_missing(&mut origins, web_ui_origin.clone());
            add_docker_loopback_aliases(&web_ui_origin, &mut origins);
        }
    }

    origins
}

fn push_origin_if_missing(origins: &mut Vec<String>, candidate: String) {
    if !origins.iter().any(|origin| origin == &candidate) {
        origins.push(candidate);
    }
}

fn canonical_origin(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if matches!(trimmed, "*" | "http://*" | "https://*") {
        return Some(trimmed.to_string());
    }

    let uri = trimmed.parse::<Uri>().ok()?;
    let scheme = uri.scheme_str()?;
    let authority = uri.authority()?;
    Some(format!("{scheme}://{authority}"))
}

fn add_docker_loopback_aliases(origin: &str, origins: &mut Vec<String>) {
    let Ok(uri) = origin.parse::<Uri>() else {
        return;
    };
    let Some(scheme) = uri.scheme_str() else {
        return;
    };
    let Some(authority) = uri.authority() else {
        return;
    };

    let host = authority.host();
    let port = authority.port_u16();
    if !matches!(
        host,
        "localhost" | "127.0.0.1" | "0.0.0.0" | "host.docker.internal" | "nodejs"
    ) {
        return;
    }

    for alias in [
        "localhost",
        "127.0.0.1",
        "0.0.0.0",
        "host.docker.internal",
        "nodejs",
    ] {
        let authority = match port {
            Some(port) => format!("{alias}:{port}"),
            None => alias.to_string(),
        };
        push_origin_if_missing(origins, format!("{scheme}://{authority}"));
    }
}

pub(crate) async fn cors_handler(
    request: Request<Body>,
    next: Next,
    policy: CorsConfig,
) -> Response {
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let requested_headers = request
        .headers()
        .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    if request.method() == Method::OPTIONS && origin.as_deref().is_some() {
        let origin = origin.expect("checked above");
        if !policy.is_allowed(&origin) {
            return StatusCode::FORBIDDEN.into_response();
        }

        let mut response = StatusCode::NO_CONTENT.into_response();
        apply_cors_headers(
            response.headers_mut(),
            &origin,
            requested_headers.as_deref(),
        );
        return response;
    }

    let mut response = next.run(request).await;
    if let Some(origin) = origin {
        if policy.is_allowed(&origin) {
            apply_cors_headers(
                response.headers_mut(),
                &origin,
                requested_headers.as_deref(),
            );
        }
    }

    response
}

pub(crate) fn apply_cors_headers(
    headers: &mut http::HeaderMap,
    origin: &str,
    requested_headers: Option<&str>,
) {
    use http::HeaderValue;

    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_str(origin).unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, PUT, PATCH, DELETE, OPTIONS"),
    );

    let mut allow_headers = "Content-Type, Authorization, X-Scryer-Language".to_string();
    if let Some(requested_headers) = requested_headers {
        let requested_headers = requested_headers.trim();
        if !requested_headers.is_empty() {
            allow_headers = format!("{}, {}", allow_headers, requested_headers);
        }
    }
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_str(&allow_headers).unwrap_or_else(|_| {
            HeaderValue::from_static("Content-Type, Authorization, X-Scryer-Language")
        }),
    );
    headers.insert(header::VARY, HeaderValue::from_static("Origin"));
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
        HeaderValue::from_static("true"),
    );
    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("86400"),
    );
}

pub(crate) async fn index_page() -> impl IntoResponse {
    let web_url =
        std::env::var("SCRYER_WEB_UI_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let base_path = BasePath::from_env();
    let graphql_url = base_path.join("/graphql");
    Html(format!(
        r#"
<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>scryer</title>
    <style>
      :root {{
        color-scheme: dark;
      }}
      body {{
        margin: 0;
        min-height: 100vh;
        font-family: Inter, system-ui, -apple-system, Segoe UI, Roboto, Helvetica, Arial, sans-serif;
        background: #0f1224;
        color: #e6edff;
        display: grid;
        place-items: center;
      }}
      main {{
        width: min(780px, 100% - 2rem);
      }}
      a {{
        color: #9fb2ff;
      }}
    </style>
  </head>
  <body>
    <main>
      <h1>scryer web UI</h1>
      <p>The SPA has moved to Next.js.</p>
      <p>
        Start the web app in <code>apps/scryer-web</code> and open
        <a href="{web_url}">{web_url}</a>.
      </p>
      <p>
        Backend endpoint: <code>{graphql_url}</code> is still served by this service.
      </p>
    </main>
  </body>
</html>
    "#,
    ))
}

pub(crate) async fn graphiql_handler() -> impl IntoResponse {
    let base_path = BasePath::from_env();
    let endpoint = base_path.join("/graphql");
    axum::response::Html(GraphiQLSource::build().endpoint(&endpoint).finish())
}

pub(crate) async fn graphql_ws_handler(
    State(state): State<AuthState>,
    protocol: GraphQLProtocol,
    ws: WebSocketUpgrade,
) -> Response {
    let schema = state.schema.clone();
    let app = state.app.clone();
    let auth_enabled = state.auth_enabled;

    let mut initial_data = Data::default();
    if !auth_enabled {
        if let Ok(user) = app.find_or_create_default_user().await {
            initial_data.insert(user);
        }
    }

    ws.protocols(ALL_WEBSOCKET_PROTOCOLS)
        .on_upgrade(move |stream| async move {
            let app_for_init = app.clone();
            GraphQLWebSocket::new(stream, schema, protocol)
                .with_data(initial_data)
                .on_connection_init(move |value: serde_json::Value| async move {
                    let mut data = Data::default();
                    if !auth_enabled {
                        return Ok(data);
                    }
                    let token = value
                        .get("Authorization")
                        .and_then(|v| v.as_str())
                        .and_then(|raw| {
                            let stripped = raw
                                .strip_prefix("Bearer ")
                                .or_else(|| raw.strip_prefix("bearer "))?;
                            Some(stripped.trim())
                        });
                    if let Some(token) = token {
                        match app_for_init.authenticate_token(token).await {
                            Ok(user) => {
                                data.insert(user);
                            }
                            Err(e) => {
                                return Err(async_graphql::Error::new(format!(
                                    "authentication failed: {e}"
                                )));
                            }
                        }
                    }
                    Ok(data)
                })
                .serve()
                .await;
        })
}

#[derive(Clone)]
pub(crate) struct AuthState {
    pub(crate) app: AppUseCase,
    pub(crate) schema: scryer_interface::ApiSchema,
    pub(crate) auth_enabled: bool,
}

/// GraphQL handler that returns a streaming response body.
///
/// When the client disconnects (e.g. via `AbortController.abort()` in the browser),
/// hyper stops polling this body stream, which drops the `execute_batch` future.
/// This cancels the entire resolver chain — including any outbound reqwest call to
/// SMG — so the cancellation propagates all the way through to the database query.
pub(crate) async fn graphql_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    body: async_graphql_axum::GraphQLBatchRequest,
) -> Response {
    let actor = resolve_actor(&state, &headers).await;
    let batch = body.into_inner();
    let batch = if let Some(user) = actor {
        match batch {
            async_graphql::BatchRequest::Single(req) => {
                async_graphql::BatchRequest::Single(req.data(user))
            }
            async_graphql::BatchRequest::Batch(reqs) => async_graphql::BatchRequest::Batch(
                reqs.into_iter().map(|req| req.data(user.clone())).collect(),
            ),
        }
    } else {
        batch
    };

    // Wrap execution in a single-item stream so the future is dropped (cancelled)
    // when hyper detects the client has disconnected.
    let schema = state.schema.clone();
    let body_stream = futures_util::stream::once(async move {
        let batch_response = schema.execute_batch(batch).await;
        Ok::<_, std::io::Error>(
            serde_json::to_vec(&batch_response).unwrap_or_else(|_| b"{}".to_vec()),
        )
    });

    Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from_stream(body_stream))
        .unwrap()
}

async fn resolve_actor(state: &AuthState, headers: &HeaderMap) -> Option<scryer_domain::User> {
    if state.auth_enabled {
        let token = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_bearer_token);
        match token {
            Some(t) => state.app.authenticate_token(t).await.ok(),
            None => None,
        }
    } else {
        state.app.find_or_create_default_user().await.ok()
    }
}

pub(crate) fn parse_bearer_token(raw: &str) -> Option<&str> {
    let mut parts = raw.split_whitespace();
    let scheme = parts.next()?;
    let token = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if scheme.eq_ignore_ascii_case("bearer") {
        Some(token)
    } else {
        None
    }
}

pub(crate) async fn resolve_actor_with_entitlement(
    app_use_case: &AppUseCase,
    auth_enabled: bool,
    headers: &HeaderMap,
    required_entitlement: Entitlement,
) -> Result<String, AppError> {
    if !auth_enabled {
        let actor = app_use_case.find_or_create_default_user().await?;
        return Ok(actor.id);
    }

    let Some(auth_header) = headers.get(header::AUTHORIZATION) else {
        return Err(AppError::Unauthorized("authorization required".into()));
    };

    let raw = auth_header
        .to_str()
        .map_err(|_| AppError::Unauthorized("invalid authorization header".into()))?;
    let token = parse_bearer_token(raw)
        .ok_or_else(|| AppError::Unauthorized("invalid authorization header".into()))?;
    let actor = app_use_case.authenticate_token(token).await?;

    if !actor.has_entitlement(&required_entitlement) {
        return Err(AppError::Unauthorized(
            "authenticated user does not have required entitlement".into(),
        ));
    }

    Ok(actor.id)
}

pub(crate) async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({"status": "ok"}))
}

pub(crate) fn map_app_error(error: AppError) -> Response {
    match error {
        AppError::Unauthorized(message) => (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse { error: message }),
        )
            .into_response(),
        AppError::Validation(message) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: message }),
        )
            .into_response(),
        AppError::NotFound(message) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse { error: message }),
        )
            .into_response(),
        AppError::Repository(message) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: message }),
        )
            .into_response(),
    }
}
