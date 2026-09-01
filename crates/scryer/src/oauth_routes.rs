use axum::Form;
use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use url::Url;

use scryer_application::{
    AppError, AppUseCase, JwtSessionScope, OAUTH_JELLYFIN_LINK_SCOPE, OAUTH_LIBRARY_SCOPE,
    OAuthAuthorizationSource,
};
use scryer_interface::context::AuthRuntimeStateHandle;

use crate::base_path::BasePath;
use crate::middleware::parse_bearer_token;

#[derive(Clone)]
pub(crate) struct OAuthRouteState {
    pub(crate) app: AppUseCase,
    pub(crate) base_path: BasePath,
    pub(crate) auth_runtime: AuthRuntimeStateHandle,
}

pub(crate) fn oauth_router(state: OAuthRouteState) -> Router {
    Router::new()
        .route("/oauth/authorize/decision", post(oauth_authorize_decision))
        .route("/oauth/token", post(oauth_token))
        .route("/oauth/revoke", post(oauth_revoke))
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth_metadata),
        )
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuthAuthorizeDecisionRequest {
    approved: bool,
    response_type: String,
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
    scope: Option<String>,
    state: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OAuthAuthorizeDecisionResponse {
    redirect_uri: String,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenRequest {
    grant_type: Option<String>,
    client_id: Option<String>,
    code: Option<String>,
    redirect_uri: Option<String>,
    code_verifier: Option<String>,
    refresh_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthRevokeRequest {
    token: Option<String>,
    #[serde(rename = "token_type_hint")]
    _token_type_hint: Option<String>,
}

#[derive(Debug, Serialize)]
struct OAuthTokenResponse {
    access_token: String,
    token_type: &'static str,
    expires_in: i64,
    refresh_token: String,
    scope: String,
}

#[derive(Debug, Serialize)]
struct OAuthErrorResponse {
    error: &'static str,
    error_description: String,
}

#[derive(Debug, Serialize)]
struct OAuthMetadataResponse {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    revocation_endpoint: String,
    response_types_supported: Vec<&'static str>,
    grant_types_supported: Vec<&'static str>,
    scopes_supported: Vec<&'static str>,
    token_endpoint_auth_methods_supported: Vec<&'static str>,
    revocation_endpoint_auth_methods_supported: Vec<&'static str>,
    code_challenge_methods_supported: Vec<&'static str>,
}

async fn oauth_authorize_decision(
    State(state): State<OAuthRouteState>,
    headers: HeaderMap,
    Json(input): Json<OAuthAuthorizeDecisionRequest>,
) -> Response {
    match oauth_authorize_decision_inner(state, headers, input).await {
        Ok(response) => Json(response).into_response(),
        Err(response) => response,
    }
}

async fn oauth_authorize_decision_inner(
    state: OAuthRouteState,
    headers: HeaderMap,
    input: OAuthAuthorizeDecisionRequest,
) -> Result<OAuthAuthorizeDecisionResponse, Response> {
    let authless_authorization = !state.auth_runtime.snapshot().effective_form_login_enabled;
    let (user, authorization_source, auth_session_version, security_action_verified_until) =
        if authless_authorization {
            let user = state
                .app
                .find_or_create_default_user()
                .await
                .map_err(oauth_app_error)?;
            let auth_session_version = state
                .app
                .current_actor_auth_session_version(&user)
                .await
                .map_err(oauth_app_error)?;
            (
                user,
                OAuthAuthorizationSource::Authless,
                auth_session_version,
                None,
            )
        } else {
            let token = bearer_token_from_headers(&headers).ok_or_else(|| {
                oauth_error(
                    StatusCode::UNAUTHORIZED,
                    "invalid_request",
                    "authorization requires a logged-in Scryer session",
                )
            })?;
            let (user, claims) = state
                .app
                .authenticate_token_with_claims(token)
                .await
                .map_err(|_| {
                    oauth_error(
                        StatusCode::UNAUTHORIZED,
                        "invalid_request",
                        "invalid session",
                    )
                })?;
            if claims.session_scope != JwtSessionScope::Full
                || claims.is_oauth_access_token()
                || AppUseCase::is_reserved_recovery_username(&user.username)
            {
                return Err(oauth_error(
                    StatusCode::FORBIDDEN,
                    "access_denied",
                    "this session cannot authorize OAuth clients",
                ));
            }
            (
                user,
                OAuthAuthorizationSource::Authenticated,
                claims.auth_session_version,
                claims.security_action_verified_until,
            )
        };
    state
        .app
        .validate_oauth_redirect_uri(&input.client_id, &input.redirect_uri)
        .await
        .map_err(oauth_validation_error)?;
    if input.response_type != "code" {
        return Ok(OAuthAuthorizeDecisionResponse {
            redirect_uri: oauth_redirect_error(
                &input.redirect_uri,
                "unsupported_response_type",
                "only response_type=code is supported",
                input.state.as_deref(),
            )
            .map_err(|response| *response)?,
        });
    }
    let scope = match state.app.validate_oauth_scope(input.scope.as_deref()) {
        Ok(scope) => scope,
        Err(err) => {
            return Ok(OAuthAuthorizeDecisionResponse {
                redirect_uri: oauth_redirect_error(
                    &input.redirect_uri,
                    "invalid_scope",
                    &oauth_error_description(&err),
                    input.state.as_deref(),
                )
                .map_err(|response| *response)?,
            });
        }
    };
    if !input.approved {
        return Ok(OAuthAuthorizeDecisionResponse {
            redirect_uri: oauth_redirect_error(
                &input.redirect_uri,
                "access_denied",
                "authorization was denied",
                input.state.as_deref(),
            )
            .map_err(|response| *response)?,
        });
    }
    if authorization_source == OAuthAuthorizationSource::Authenticated
        && security_action_verified_until
            .is_none_or(|verified_until| verified_until <= chrono::Utc::now().timestamp())
    {
        return Err(oauth_error(
            StatusCode::UNAUTHORIZED,
            "reauthentication_required",
            "sign in again before authorizing this OAuth client",
        ));
    }
    let code = state
        .app
        .create_oauth_authorization_code(
            &user,
            &input.client_id,
            &input.redirect_uri,
            &scope,
            &input.code_challenge,
            &input.code_challenge_method,
            authorization_source,
            auth_session_version.as_deref(),
        )
        .await
        .map_err(oauth_validation_error)?;
    let mut redirect = Url::parse(&input.redirect_uri).map_err(|_| {
        oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "invalid redirect_uri",
        )
    })?;
    redirect.query_pairs_mut().append_pair("code", &code.code);
    if let Some(state_value) = input.state.as_deref() {
        redirect.query_pairs_mut().append_pair("state", state_value);
    }
    Ok(OAuthAuthorizeDecisionResponse {
        redirect_uri: redirect.to_string(),
    })
}

async fn oauth_token(
    State(state): State<OAuthRouteState>,
    Form(input): Form<OAuthTokenRequest>,
) -> Response {
    let Some(grant_type) = input
        .grant_type
        .as_deref()
        .filter(|value| !value.is_empty())
    else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "grant_type is required",
        );
    };
    let Some(client_id) = input.client_id.as_deref().filter(|value| !value.is_empty()) else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "client_id is required",
        );
    };
    match state.app.oauth_client_info(client_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_client",
                "unknown OAuth client",
            );
        }
        Err(error) => return oauth_app_error(error),
    }
    let result = match grant_type {
        "authorization_code" => {
            let Some(code) = input.code.as_deref() else {
                return oauth_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "code is required",
                );
            };
            let Some(redirect_uri) = input.redirect_uri.as_deref() else {
                return oauth_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "redirect_uri is required",
                );
            };
            let Some(code_verifier) = input.code_verifier.as_deref() else {
                return oauth_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "code_verifier is required",
                );
            };
            let authless_codes_allowed =
                !state.auth_runtime.snapshot().effective_form_login_enabled;
            state
                .app
                .exchange_oauth_authorization_code(
                    client_id,
                    code,
                    redirect_uri,
                    code_verifier,
                    authless_codes_allowed,
                )
                .await
        }
        "refresh_token" => {
            let Some(refresh_token) = input.refresh_token.as_deref() else {
                return oauth_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "refresh_token is required",
                );
            };
            let authless_grants_allowed =
                !state.auth_runtime.snapshot().effective_form_login_enabled;
            state
                .app
                .refresh_oauth_token(client_id, refresh_token, authless_grants_allowed)
                .await
        }
        _ => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "unsupported_grant_type",
                "only authorization_code and refresh_token are supported",
            );
        }
    };
    match result {
        Ok(pair) => oauth_token_response(OAuthTokenResponse {
            access_token: pair.access_token,
            token_type: "Bearer",
            expires_in: pair.expires_in,
            refresh_token: pair.refresh_token,
            scope: pair.scope,
        }),
        Err(err) => oauth_app_error(err),
    }
}

async fn oauth_revoke(
    State(state): State<OAuthRouteState>,
    Form(input): Form<OAuthRevokeRequest>,
) -> Response {
    let Some(token) = input.token.as_deref().filter(|value| !value.is_empty()) else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "token is required",
        );
    };
    if let Err(err) = state.app.revoke_oauth_refresh_token(token).await {
        tracing::debug!(error = %err, "OAuth token revocation did not match an active refresh token");
    }
    StatusCode::OK.into_response()
}

async fn oauth_metadata(State(state): State<OAuthRouteState>, headers: HeaderMap) -> Response {
    let issuer = match oauth_issuer_origin(&headers) {
        Ok(origin) => origin,
        Err(response) => return *response,
    };
    Json(OAuthMetadataResponse {
        issuer: issuer.clone(),
        authorization_endpoint: absolute_oauth_url(&issuer, &state.base_path, "/oauth/authorize"),
        token_endpoint: absolute_oauth_url(&issuer, &state.base_path, "/oauth/token"),
        revocation_endpoint: absolute_oauth_url(&issuer, &state.base_path, "/oauth/revoke"),
        response_types_supported: vec!["code"],
        grant_types_supported: vec!["authorization_code", "refresh_token"],
        scopes_supported: vec![OAUTH_LIBRARY_SCOPE, OAUTH_JELLYFIN_LINK_SCOPE],
        token_endpoint_auth_methods_supported: vec!["none"],
        revocation_endpoint_auth_methods_supported: vec!["none"],
        code_challenge_methods_supported: vec!["S256"],
    })
    .into_response()
}

fn bearer_token_from_headers(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    parse_bearer_token(value)
}

fn oauth_redirect_error(
    redirect_uri: &str,
    error: &str,
    description: &str,
    state: Option<&str>,
) -> Result<String, Box<Response>> {
    let mut redirect = Url::parse(redirect_uri).map_err(|_| {
        Box::new(oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "invalid redirect_uri",
        ))
    })?;
    redirect.query_pairs_mut().append_pair("error", error);
    redirect
        .query_pairs_mut()
        .append_pair("error_description", description);
    if let Some(state) = state {
        redirect.query_pairs_mut().append_pair("state", state);
    }
    Ok(redirect.to_string())
}

fn oauth_validation_error(err: AppError) -> Response {
    match err {
        AppError::Unauthorized(error) => {
            tracing::debug!(error = %error, "OAuth validation rejected an authorization grant");
            oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_grant",
                "OAuth grant is invalid",
            )
        }
        AppError::Validation(message) if message == "invalid OAuth scope" => {
            oauth_error(StatusCode::BAD_REQUEST, "invalid_scope", message)
        }
        AppError::Validation(message) => {
            oauth_error(StatusCode::BAD_REQUEST, "invalid_request", message)
        }
        other => {
            tracing::error!(error = %other, "OAuth validation failed unexpectedly");
            oauth_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "OAuth request could not be processed",
            )
        }
    }
}

fn oauth_app_error(err: AppError) -> Response {
    match err {
        AppError::Unauthorized(error) => {
            tracing::debug!(error = %error, "OAuth grant was rejected");
            oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_grant",
                "OAuth grant is invalid",
            )
        }
        AppError::Validation(message) if message == "invalid OAuth scope" => {
            oauth_error(StatusCode::BAD_REQUEST, "invalid_scope", message)
        }
        AppError::Validation(error) => {
            tracing::debug!(error = %error, "OAuth request validation failed");
            oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "OAuth request is invalid",
            )
        }
        other => {
            tracing::error!(error = %other, "OAuth request failed unexpectedly");
            oauth_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "OAuth request could not be processed",
            )
        }
    }
}

fn oauth_token_response(body: OAuthTokenResponse) -> Response {
    let mut response = Json(body).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

fn oauth_error_description(err: &AppError) -> String {
    match err {
        AppError::Validation(message) | AppError::Unauthorized(message) => message.clone(),
        other => other.to_string(),
    }
}

fn oauth_error(
    status: StatusCode,
    error: &'static str,
    description: impl Into<String>,
) -> Response {
    (
        status,
        Json(OAuthErrorResponse {
            error,
            error_description: description.into(),
        }),
    )
        .into_response()
}

const SCRYER_PUBLIC_URL_ENV: &str = "SCRYER_PUBLIC_URL";

fn oauth_issuer_origin(headers: &HeaderMap) -> Result<String, Box<Response>> {
    if let Ok(public_url) = std::env::var(SCRYER_PUBLIC_URL_ENV)
        && !public_url.trim().is_empty()
    {
        return parse_oauth_origin(public_url.trim(), SCRYER_PUBLIC_URL_ENV);
    }

    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("http")
        .trim();
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("localhost")
        .trim();
    if !safe_forwarded_value(proto) || !safe_forwarded_value(host) {
        return Err(Box::new(oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "invalid request origin",
        )));
    }
    if !matches!(proto, "http" | "https") {
        return Err(Box::new(oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "invalid request scheme",
        )));
    }
    parse_oauth_origin(&format!("{proto}://{host}"), "request origin")
}

fn parse_oauth_origin(value: &str, source: &str) -> Result<String, Box<Response>> {
    let url = Url::parse(value).map_err(|_| {
        Box::new(oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            format!("invalid {source}"),
        ))
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Box::new(oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            format!("invalid {source}"),
        )));
    }
    Ok(url.origin().ascii_serialization())
}

fn safe_forwarded_value(value: &str) -> bool {
    !value.is_empty() && !value.contains(',') && !value.chars().any(char::is_control)
}

fn absolute_oauth_url(issuer: &str, base_path: &BasePath, suffix: &str) -> String {
    format!("{}{}", issuer.trim_end_matches('/'), base_path.join(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_error_uses_standard_fields_and_preserves_state() {
        let redirect = oauth_redirect_error(
            "http://127.0.0.1:49152/callback?existing=1",
            "invalid_scope",
            "unsupported scope",
            Some("state value"),
        )
        .expect("redirect");
        let url = Url::parse(&redirect).expect("url");
        let pairs = url
            .query_pairs()
            .into_owned()
            .collect::<std::collections::HashMap<_, _>>();

        assert_eq!(pairs.get("existing").map(String::as_str), Some("1"));
        assert_eq!(
            pairs.get("error").map(String::as_str),
            Some("invalid_scope")
        );
        assert_eq!(
            pairs.get("error_description").map(String::as_str),
            Some("unsupported scope")
        );
        assert_eq!(pairs.get("state").map(String::as_str), Some("state value"));
    }

    #[test]
    fn token_response_sets_standard_no_store_headers() {
        let response = oauth_token_response(OAuthTokenResponse {
            access_token: "access".to_string(),
            token_type: "Bearer",
            expires_in: 300,
            refresh_token: "refresh".to_string(),
            scope: OAUTH_LIBRARY_SCOPE.to_string(),
        });

        assert_eq!(
            response
                .headers()
                .get(header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            Some("no-store")
        );
        assert_eq!(
            response
                .headers()
                .get(header::PRAGMA)
                .and_then(|value| value.to_str().ok()),
            Some("no-cache")
        );
    }

    #[test]
    fn metadata_endpoint_urls_include_base_path() {
        let base_path = BasePath::from_raw(Some("/scryer"));

        assert_eq!(
            absolute_oauth_url("https://scryer.example", &base_path, "/oauth/token"),
            "https://scryer.example/scryer/oauth/token"
        );
    }

    #[test]
    fn issuer_origin_rejects_unsafe_forwarded_values() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https,http"));
        headers.insert(header::HOST, HeaderValue::from_static("scryer.example"));

        assert!(oauth_issuer_origin(&headers).is_err());

        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        headers.insert(header::HOST, HeaderValue::from_static("scryer.example"));

        assert_eq!(
            oauth_issuer_origin(&headers).expect("safe origin"),
            "https://scryer.example"
        );
    }
}
