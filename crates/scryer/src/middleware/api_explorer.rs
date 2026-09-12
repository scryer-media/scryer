use super::*;
use tokio::sync::watch;

pub(super) const HEADER: &str = "x-scryer-api-explorer-mode";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AccessMode {
    ApiKey,
    OAuth,
}

impl AccessMode {
    pub(super) fn parse(value: &str) -> AppResult<Self> {
        match value {
            "api-key" => Ok(Self::ApiKey),
            "oauth" => Ok(Self::OAuth),
            _ => Err(AppError::Validation(
                "Invalid API explorer access mode".into(),
            )),
        }
    }

    pub(super) fn audit_source(self) -> &'static str {
        match self {
            Self::ApiKey => "api_explorer_api_key",
            Self::OAuth => "api_explorer_oauth",
        }
    }

    /// Shared with real credentials so the explorer follows the same RBAC policy.
    pub(super) fn restrict(self, user: &mut scryer_domain::User) {
        user.authorization.actor_capabilities = ActorCapabilityMask::NONE;
        if self == Self::OAuth {
            user.authorization.app = AppPermissionMask::NONE;
        }
    }
}

pub(super) fn http_mode(headers: &HeaderMap) -> AppResult<Option<AccessMode>> {
    if headers.get_all(HEADER).iter().count() > 1 {
        return Err(AppError::Validation(
            "Duplicate API explorer access mode".into(),
        ));
    }
    headers
        .get(HEADER)
        .map(|value| {
            AccessMode::parse(
                value
                    .to_str()
                    .map_err(|_| AppError::Validation("Invalid API explorer access mode".into()))?,
            )
        })
        .transpose()
}

pub(super) async fn masquerade(
    app: &AppUseCase,
    actor: &mut ResolvedActor,
    mode: AccessMode,
) -> AppResult<()> {
    if !(actor.is_interactive_session() || actor.is_authless_default_session())
        || actor.token_claims.session_scope != JwtSessionScope::Full
    {
        return Err(AppError::Unauthorized(
            "API explorer requires a web session".into(),
        ));
    }
    require_access(app, &actor.user).await?;
    mode.restrict(&mut actor.user);
    actor.token_claims = AuthenticatedTokenClaims::default();
    actor.source = ResolvedActorSource::Explorer(mode);
    Ok(())
}

async fn require_access(app: &AppUseCase, user: &scryer_domain::User) -> AppResult<()> {
    app.require_app_permission(user, scryer_domain::AppPermission::ManageSystemSettings)
        .await?;
    if !app.api_explorer_enabled().await? {
        return Err(AppError::Unauthorized("API explorer is disabled".into()));
    }
    Ok(())
}

pub(super) fn http_error() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(serde_json::json!({
            "errors": [{
                "message": "API explorer is unavailable for this session or access mode",
                "extensions": { "code": "API_EXPLORER_UNAVAILABLE" }
            }]
        })),
    )
        .into_response()
}

pub(super) async fn validate_session(
    app: &AppUseCase,
    session: &watch::Receiver<Option<ResolvedActor>>,
) -> AppResult<()> {
    let original = session.borrow().clone();
    let Some(original) = original else {
        return Ok(());
    };
    let current = app.load_user_for_auth_payload(&original.user).await?;
    require_access(app, &current).await?;
    // End subscriptions on permission changes rather than retaining old library grants.
    if current.authorization.app != original.user.authorization.app
        || current.authorization.libraries != original.user.authorization.libraries
        || current.authorization.default_library != original.user.authorization.default_library
        || current.login_status() != original.user.login_status()
        || (original.is_interactive_session()
            && app.current_actor_auth_session_version(&current).await?
                != original.token_claims.auth_session_version)
    {
        return Err(AppError::Unauthorized(
            "API explorer session changed".into(),
        ));
    }
    Ok(())
}

/// Idle subscriptions must also end after disabling the explorer or revoking access.
pub(super) async fn until_revoked(
    app: AppUseCase,
    mut session: watch::Receiver<Option<ResolvedActor>>,
) {
    if session.wait_for(|actor| actor.is_some()).await.is_err() {
        // A normal connection never opts in. Finishing this future would close it.
        std::future::pending::<()>().await;
        return;
    }
    loop {
        if validate_session(&app, &session).await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::super::integration_test_common::TestContext;
    use super::*;
    use axum::{Router, body::to_bytes, routing::post};
    use serde_json::{Value, json};
    use tower::ServiceExt;

    async fn setup() -> (TestContext, ResolvedActor) {
        let ctx = TestContext::new().await;
        ctx.settings_store
            .batch_ensure_setting_definitions(vec![
                scryer_infrastructure_sql::types::SettingDefinitionSeed {
                    category: "ui".into(),
                    scope: "system".into(),
                    key_name: "ui.api_explorer_enabled".into(),
                    data_type: "boolean".into(),
                    default_value_json: "false".into(),
                    is_sensitive: false,
                    validation_json: None,
                },
            ])
            .await
            .unwrap();
        let user = ctx.app.find_or_create_default_user().await.unwrap();
        let actor = attach_resolved_actor(
            &ctx.app,
            user,
            AuthenticatedTokenClaims::default(),
            ResolvedActorSource::AuthlessDefault,
            None,
        )
        .await
        .unwrap();
        (ctx, actor)
    }

    async fn set_enabled(ctx: &TestContext, actor: &ResolvedActor, enabled: bool) {
        let result = ctx.schema.execute(async_graphql::Request::new(format!(
            "mutation {{ updateGeneralSettings(input: {{apiExplorerEnabled: {enabled}}}) {{ apiExplorerEnabled }} }}"
        )).data(actor.user.clone()).data(AuthlessDefaultSession)).await;
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(
            result.data.into_json().unwrap()["updateGeneralSettings"]["apiExplorerEnabled"],
            enabled
        );
    }

    #[tokio::test]
    async fn api_explorer_setting_defaults_off_and_requires_admin() {
        let (ctx, admin) = setup().await;
        assert!(!ctx.app.api_explorer_enabled().await.unwrap());
        assert!(
            masquerade(&ctx.app, &mut admin.clone(), AccessMode::ApiKey)
                .await
                .is_err()
        );
        set_enabled(&ctx, &admin, true).await;
        assert!(
            ctx.app
                .get_general_settings(&admin.user)
                .await
                .unwrap()
                .api_explorer_enabled
        );
        let mut ordinary = admin.clone();
        ordinary.user.authorization.app = AppPermissionMask::NONE;
        assert!(ctx.app.get_general_settings(&ordinary.user).await.is_err());
        assert!(
            !ctx.app
                .instance_features(&ordinary.user)
                .await
                .unwrap()
                .api_explorer_enabled
        );
        let denied = ctx.schema.execute(async_graphql::Request::new(
            "mutation { updateGeneralSettings(input: {apiExplorerEnabled: false}) { apiExplorerEnabled } }"
        ).data(ordinary.user.clone()).data(AuthlessDefaultSession)).await;
        assert!(!denied.errors.is_empty());
        assert!(ctx.app.api_explorer_enabled().await.unwrap());
        assert!(
            masquerade(&ctx.app, &mut ordinary, AccessMode::OAuth)
                .await
                .is_err()
        );
        set_enabled(&ctx, &admin, false).await;
        assert!(!ctx.app.api_explorer_enabled().await.unwrap());
    }

    #[tokio::test]
    async fn api_explorer_modes_preserve_library_grants_and_remove_session_privileges() {
        let (ctx, mut admin) = setup().await;
        set_enabled(&ctx, &admin, true).await;
        admin.user.authorization.libraries.insert(
            "visible-library".into(),
            scryer_domain::LibraryPermissionMask::VIEW,
        );
        admin.user.authorization.default_library = scryer_domain::LibraryPermissionMask::NONE;
        // System settings access need not include the catalog-wide administrator override.
        admin.user.authorization.app = AppPermissionMask::MANAGE_SYSTEM_SETTINGS;
        admin.token_claims.mfa_verified_until = Some(i64::MAX);
        for mode in [AccessMode::ApiKey, AccessMode::OAuth] {
            let mut actor = admin.clone();
            masquerade(&ctx.app, &mut actor, mode).await.unwrap();
            let mut expected = admin.user.clone();
            mode.restrict(&mut expected);
            assert_eq!(actor.user.authorization, expected.authorization);
            assert_eq!(actor.user.id, admin.user.id);
            assert_eq!(
                actor.user.authorization.libraries,
                admin.user.authorization.libraries
            );
            assert_eq!(
                actor.user.authorization.default_library,
                admin.user.authorization.default_library
            );
            assert_eq!(
                actor.user.authorization.actor_capabilities,
                ActorCapabilityMask::NONE
            );
            assert_eq!(
                actor.user.authorization.app,
                if mode == AccessMode::OAuth {
                    AppPermissionMask::NONE
                } else {
                    admin.user.authorization.app
                }
            );
            assert!(!actor.is_interactive_session());
            assert!(!actor.can_manage_api_keys());
            ctx.app
                .require_library_permission(
                    &actor.user,
                    "visible-library",
                    scryer_domain::LibraryPermission::View,
                )
                .await
                .unwrap();
            assert!(
                ctx.app
                    .require_library_permission(
                        &actor.user,
                        "other-library",
                        scryer_domain::LibraryPermission::View
                    )
                    .await
                    .is_err()
            );
            assert!(actor.mfa_verification().verified_until.is_none());
            assert_eq!(
                actor_log_context(&actor).source.as_deref(),
                Some(mode.audit_source())
            );
            let data = graphql_ws_connection_data(1, Some(actor));
            assert!(
                data.get(&std::any::TypeId::of::<InteractiveSession>())
                    .is_none()
            );
            assert!(
                data.get(&std::any::TypeId::of::<AuthlessDefaultSession>())
                    .is_none()
            );
            assert!(
                data.get(&std::any::TypeId::of::<ApiKeyManagementSession>())
                    .is_none()
            );
        }
    }

    #[tokio::test]
    async fn api_explorer_rejects_real_credentials_and_partial_sessions() {
        let (ctx, admin) = setup().await;
        set_enabled(&ctx, &admin, true).await;
        let mut api_key = admin.clone();
        api_key.source = ResolvedActorSource::ApiKey;
        let mut oauth = admin.clone();
        oauth.source = ResolvedActorSource::AuthenticatedToken;
        oauth.token_claims.oauth_client_id = Some("client".into());
        oauth.token_claims.oauth_grant_id = Some("grant".into());
        let mut partial = admin.clone();
        partial.source = ResolvedActorSource::AuthenticatedToken;
        partial.token_claims.session_scope = JwtSessionScope::MfaEnrollment;
        for mut actor in [api_key, oauth, partial] {
            assert!(
                masquerade(&ctx.app, &mut actor, AccessMode::OAuth)
                    .await
                    .is_err()
            );
        }
    }

    #[test]
    fn api_explorer_rejects_invalid_and_duplicate_markers() {
        let mut headers = HeaderMap::new();
        assert_eq!(http_mode(&headers).unwrap(), None);
        headers.insert(HEADER, "oauth".parse().unwrap());
        assert_eq!(http_mode(&headers).unwrap(), Some(AccessMode::OAuth));
        headers.append(HEADER, "api-key".parse().unwrap());
        assert!(http_mode(&headers).is_err());
        headers.insert(HEADER, "admin".parse().unwrap());
        assert!(http_mode(&headers).is_err());
    }

    async fn http_query(
        ctx: &TestContext,
        query: &str,
        mode: Option<&str>,
        proof: bool,
    ) -> (StatusCode, Value) {
        let state = AuthState {
            app: ctx.app.clone(),
            schema: ctx.schema.clone(),
            auth_runtime: ctx.auth_runtime.clone(),
            rate_limiter: ScryerRateLimiter::from_env(),
            ws_origin_policy: WebSocketOriginPolicy::default(),
            authless_web_client_proof: AuthlessWebClientProofState::new(),
        };
        let mut request = Request::builder()
            .method("POST")
            .uri("/graphql")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(mode) = mode {
            request = request.header(HEADER, mode);
        }
        if proof {
            let (nonce, proof, _) = state.authless_web_client_proof.issue().unwrap();
            request = request.header(AUTHLESS_WEB_CLIENT_HEADER, proof).header(
                header::COOKIE,
                format!("{AUTHLESS_WEB_CLIENT_COOKIE}={nonce}"),
            );
        }
        let mut request = request
            .body(Body::from(json!({"query": query}).to_string()))
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 3000))));
        let response = Router::new()
            .route("/graphql", post(graphql_handler))
            .with_state(state)
            .oneshot(request)
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn api_explorer_http_enforces_gate_proof_and_mode_without_changing_ordinary_api() {
        let (ctx, admin) = setup().await;
        let query = "{ generalSettings { apiExplorerEnabled } }";
        assert_eq!(
            http_query(&ctx, query, Some("api-key"), true).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(http_query(&ctx, query, None, true).await.0, StatusCode::OK);
        set_enabled(&ctx, &admin, true).await;
        assert_eq!(
            http_query(&ctx, query, Some("api-key"), false).await.0,
            StatusCode::FORBIDDEN
        );
        let (status, body) = http_query(&ctx, query, Some("api-key"), true).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["data"]["generalSettings"]["apiExplorerEnabled"], true,
            "{body}"
        );
        let (status, body) = http_query(&ctx, query, Some("oauth"), true).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body["errors"]
                .as_array()
                .is_some_and(|errors| !errors.is_empty()),
            "{body}"
        );
        let (_, body) = http_query(
            &ctx,
            "{ __schema { queryType { name } } }",
            Some("oauth"),
            true,
        )
        .await;
        assert_eq!(body["data"]["__schema"]["queryType"]["name"], "QueryRoot");
        assert_eq!(
            http_query(&ctx, query, Some("admin"), true).await.0,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn api_explorer_idle_session_ends_on_disablement_or_changed_grants() {
        let (ctx, admin) = setup().await;
        set_enabled(&ctx, &admin, true).await;
        let (_sender, session) = watch::channel(Some(admin.clone()));
        validate_session(&ctx.app, &session).await.unwrap();
        let (sender, ordinary_session) = watch::channel(None);
        drop(sender);
        assert!(
            tokio::time::timeout(
                Duration::from_millis(20),
                until_revoked(ctx.app.clone(), ordinary_session)
            )
            .await
            .is_err()
        );
        let mut stale = admin.clone();
        stale.user.authorization.libraries.insert(
            "revoked-library".into(),
            scryer_domain::LibraryPermissionMask::VIEW,
        );
        let (_sender, stale_session) = watch::channel(Some(stale));
        assert!(validate_session(&ctx.app, &stale_session).await.is_err());
        let mut revocation = Box::pin(until_revoked(ctx.app.clone(), session.clone()));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut revocation)
                .await
                .is_err()
        );
        set_enabled(&ctx, &admin, false).await;
        tokio::time::timeout(Duration::from_secs(2), revocation)
            .await
            .unwrap();
        set_enabled(&ctx, &admin, true).await;
        scryer_application::LibraryRepository::set_app_permission_mask_for_user(
            &ctx.libraries,
            &admin.user.id,
            AppPermissionMask::NONE,
        )
        .await
        .unwrap();
        assert!(validate_session(&ctx.app, &session).await.is_err());
    }
}
