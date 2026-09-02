//! Download the selected indexer-search releases to the browser (D17, FR-028).
//!
//! One release answers with its own `.nzb`/`.torrent`, several with one
//! `tar.gz`. The route exists instead of a GraphQL mutation because the payload
//! is a file: it is authenticated like the media-server avatar proxy and shaped
//! like the backup download, both of which already stream bytes out of axum.

use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use scryer_application::{AppError, InteractiveSearchArtifactTarget, JwtSessionScope};
use serde::Deserialize;
use std::net::SocketAddr;

use crate::middleware::{AuthState, map_app_error, resolve_actor};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadArtifactsRequest {
    releases: Vec<DownloadArtifactRelease>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadArtifactRelease {
    search_id: String,
    download_url: String,
}

pub(crate) async fn download_indexer_search_artifacts_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    body: Bytes,
) -> Response {
    let Ok(Some(actor)) = resolve_actor(&state, &headers, Some(remote_addr)).await else {
        return unauthorized();
    };
    // A step-up or enrollment session must not be able to pull release files.
    if actor.token_claims.session_scope != JwtSessionScope::Full {
        return unauthorized();
    }

    let request = match serde_json::from_slice::<DownloadArtifactsRequest>(&body) {
        Ok(request) => request,
        Err(error) => {
            return map_app_error(AppError::Validation(format!(
                "invalid release download request: {error}"
            )));
        }
    };

    let targets = request
        .releases
        .into_iter()
        .map(|release| InteractiveSearchArtifactTarget {
            search_id: release.search_id,
            download_url: release.download_url,
        })
        .collect::<Vec<_>>();

    let bundle = match state
        .app
        .download_interactive_search_artifacts(&actor.user, &targets)
        .await
    {
        Ok(bundle) => bundle,
        // A permission refusal is not an authentication failure: `map_app_error`
        // would answer 401, which the web client reads as an expired session and
        // turns into a logout. The avatar proxy separates the two the same way.
        Err(AppError::Unauthorized(_)) => {
            return Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(Body::empty())
                .expect("an empty forbidden response is always valid");
        }
        Err(error) => return map_app_error(error),
    };

    let mut response = Response::builder().status(StatusCode::OK);
    if let Ok(value) = HeaderValue::from_str(&bundle.content_type) {
        response = response.header(header::CONTENT_TYPE, value);
    }
    if let Ok(value) = HeaderValue::from_str(&content_disposition(&bundle.file_name)) {
        response = response.header(header::CONTENT_DISPOSITION, value);
    }
    response
        .header(header::CONTENT_LENGTH, bundle.bytes.len())
        // Release files are actor-scoped and single-use; nothing may cache them.
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(bundle.bytes))
        .unwrap_or_else(|_| unauthorized())
}

fn unauthorized() -> Response {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .body(Body::empty())
        .expect("an empty unauthorized response is always valid")
}

/// `attachment` with both filename forms (RFC 6266): the quoted one is ASCII so
/// every client can read it, and `filename*` carries the real release name,
/// which is routinely non-ASCII.
fn content_disposition(file_name: &str) -> String {
    let ascii = file_name
        .chars()
        .map(|character| {
            let quotable = character.is_ascii_graphic() || character == ' ';
            if quotable && character != '\\' && character != '"' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let encoded = utf8_percent_encode(file_name, NON_ALPHANUMERIC);
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::integration_test_common as common;
    use crate::middleware::{AuthlessWebClientProofState, WebSocketOriginPolicy};
    use crate::rate_limit::ScryerRateLimiter;

    use axum::Router;
    use axum::routing::post;
    use scryer_domain::AppPermissionMask;
    use std::net::Ipv4Addr;
    use tower::ServiceExt as _;

    #[test]
    fn content_disposition_carries_an_ascii_fallback_and_the_real_name() {
        let header = content_disposition("Say \"Hi\"\\Now.nzb");
        assert!(
            header.starts_with("attachment; filename=\"Say _Hi__Now.nzb\"; filename*=UTF-8''"),
            "{header}"
        );

        let header = content_disposition("東京.nzb");
        assert!(header.contains("filename=\"__.nzb\""), "{header}");
        assert!(
            header.ends_with("filename*=UTF-8''%E6%9D%B1%E4%BA%AC%2Enzb"),
            "{header}"
        );
    }

    #[tokio::test]
    async fn indexer_search_artifacts_route_gates_on_a_full_session_and_the_settings_permission() {
        let context = common::TestContext::new().await;
        let admin = context
            .app
            .find_or_create_default_user()
            .await
            .expect("default administrator");
        let ordinary = context
            .app
            .create_user(
                &admin,
                "artifact-ordinary".into(),
                "ordinary-password".into(),
                AppPermissionMask::NONE,
                Vec::new(),
            )
            .await
            .expect("create ordinary actor");
        let ordinary_token = context
            .app
            .issue_access_token(&ordinary)
            .await
            .expect("issue ordinary token");
        let admin_token = context
            .app
            .issue_access_token(&admin)
            .await
            .expect("issue administrator token");

        let state = AuthState {
            app: context.app.clone(),
            schema: context.schema.clone(),
            // The shared fixture leaves authless local access on, which would
            // resolve a default administrator for an anonymous request.
            auth_runtime: scryer_interface::context::AuthRuntimeStateHandle::new(
                scryer_interface::context::AuthRuntimeStateSnapshot {
                    form_login_enabled: true,
                    skip_login_for_local_ips: false,
                    effective_form_login_enabled: true,
                    webauthn_configured: false,
                    passkey_enabled: false,
                    env_override_active: false,
                    env_override_description: None,
                    epoch: 1,
                },
            ),
            rate_limiter: ScryerRateLimiter::from_env(),
            ws_origin_policy: WebSocketOriginPolicy::default(),
            authless_web_client_proof: AuthlessWebClientProofState::new(),
        };
        let router = Router::new()
            .route(
                "/api/indexer-search/artifacts",
                post(download_indexer_search_artifacts_handler),
            )
            .with_state(state);
        let request = |token: Option<&str>, body: &'static str| {
            let mut request = axum::http::Request::builder()
                .method("POST")
                .uri("/api/indexer-search/artifacts")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .expect("artifact request");
            if let Some(token) = token {
                request.headers_mut().insert(
                    header::AUTHORIZATION,
                    HeaderValue::from_str(&format!("Bearer {token}")).expect("authorization"),
                );
            }
            request
                .extensions_mut()
                .insert(ConnectInfo(SocketAddr::from((Ipv4Addr::LOCALHOST, 3000))));
            request
        };
        let status = |token: Option<&'static str>, body: &'static str| {
            let router = router.clone();
            async move {
                router
                    .oneshot(request(token, body))
                    .await
                    .expect("artifact response")
                    .status()
            }
        };

        const BODY: &str =
            r#"{"releases":[{"searchId":"job-1","downloadUrl":"https://example.invalid/a.nzb"}]}"#;
        assert_eq!(status(None, BODY).await, StatusCode::UNAUTHORIZED);
        assert_eq!(status(Some("invalid-token"), BODY).await, StatusCode::UNAUTHORIZED);

        // A valid session with the wrong permission never reaches the search.
        let ordinary_token: &'static str = Box::leak(ordinary_token.into_boxed_str());
        assert_eq!(status(Some(ordinary_token), BODY).await, StatusCode::FORBIDDEN);

        let admin_token: &'static str = Box::leak(admin_token.into_boxed_str());
        assert_eq!(
            status(Some(admin_token), "not json").await,
            StatusCode::BAD_REQUEST
        );
        // Past the gates, an unknown search is a 404 like every other lookup.
        assert_eq!(status(Some(admin_token), BODY).await, StatusCode::NOT_FOUND);
    }
}
