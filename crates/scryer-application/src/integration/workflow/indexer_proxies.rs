impl AppUseCase {
    pub async fn list_indexer_proxy_configs(
        &self,
        actor: &User,
    ) -> AppResult<Vec<scryer_domain::IndexerProxyConfig>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.services
            .integrations
            .indexer_proxy_configs
            .list(None)
            .await
    }

    pub async fn create_indexer_proxy_config(
        &self,
        actor: &User,
        input: NewIndexerProxyConfig,
    ) -> AppResult<scryer_domain::IndexerProxyConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let name = normalize_indexer_proxy_name(&input.name)?;
        let base_url = normalize_indexer_proxy_base_url(&input.base_url)?;
        let request_timeout_seconds =
            validate_indexer_proxy_timeout(input.request_timeout_seconds.unwrap_or(60))?;
        let now = Utc::now();
        let config = scryer_domain::IndexerProxyConfig {
            id: Id::new().0,
            name,
            provider_type: input.provider_type,
            protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
            base_url,
            request_timeout_seconds,
            is_enabled: input.is_enabled,
            last_health_status: Some(scryer_domain::IndexerProxyHealthStatus::Unknown),
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
        };
        self.services
            .integrations
            .indexer_proxy_configs
            .create(config)
            .await
    }

    pub async fn update_indexer_proxy_config(
        &self,
        actor: &User,
        update: IndexerProxyConfigUpdate,
    ) -> AppResult<scryer_domain::IndexerProxyConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let id = update.id.trim();
        if id.is_empty() {
            return Err(AppError::Validation(
                "indexer proxy config id is required".into(),
            ));
        }
        if !update.has_changes() {
            return Err(AppError::Validation(
                "at least one indexer proxy field must be provided".into(),
            ));
        }

        let mut config = self
            .services
            .integrations
            .indexer_proxy_configs
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("indexer proxy config '{id}' not found")))?;
        if let Some(name) = update.name {
            config.name = normalize_indexer_proxy_name(&name)?;
        }
        if let Some(base_url) = update.base_url {
            config.base_url = normalize_indexer_proxy_base_url(&base_url)?;
        }
        if let Some(timeout) = update.request_timeout_seconds {
            config.request_timeout_seconds = validate_indexer_proxy_timeout(timeout)?;
        }
        if update.is_enabled == Some(false) && config.is_enabled {
            let assigned_count = self
                .services
                .integrations
                .indexer_configs
                .list(None)
                .await?
                .into_iter()
                .filter(|indexer| {
                    indexer.is_enabled && indexer.indexer_proxy_config_id.as_deref() == Some(id)
                })
                .count();
            if assigned_count > 0 {
                return Err(AppError::Validation(format!(
                    "indexer proxy config is assigned to {assigned_count} enabled indexer(s)"
                )));
            }
        }
        if let Some(is_enabled) = update.is_enabled {
            config.is_enabled = is_enabled;
        }
        config.updated_at = Utc::now();

        self.services
            .integrations
            .indexer_proxy_configs
            .update(config)
            .await
    }

    pub async fn delete_indexer_proxy_config(&self, actor: &User, id: &str) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let id = id.trim();
        if id.is_empty() {
            return Err(AppError::Validation(
                "indexer proxy config id is required".into(),
            ));
        }
        let assigned = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?
            .into_iter()
            .any(|indexer| indexer.indexer_proxy_config_id.as_deref() == Some(id));
        if assigned {
            return Err(AppError::Validation(
                "indexer proxy config is assigned to one or more indexers".into(),
            ));
        }
        self.services
            .integrations
            .indexer_proxy_configs
            .delete(id)
            .await
    }

    pub async fn test_indexer_proxy_config(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<IndexerProxyTestResult> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let id = id.trim();
        if id.is_empty() {
            return Err(AppError::Validation(
                "indexer proxy config id is required".into(),
            ));
        }
        let config = self
            .services
            .integrations
            .indexer_proxy_configs
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("indexer proxy config '{id}' not found")))?;

        let started = std::time::Instant::now();
        let result = probe_solver_health(&config).await;
        let duration_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        let test_result = match result {
            Ok(message) => IndexerProxyTestResult {
                ok: true,
                status: scryer_domain::IndexerProxyHealthStatus::Healthy,
                message: Some(message),
                duration_ms: Some(duration_ms),
            },
            Err(error) => IndexerProxyTestResult {
                ok: false,
                status: scryer_domain::IndexerProxyHealthStatus::Unhealthy,
                message: Some(crate::challenge_solver::sanitize_indexer_proxy_error(
                    &error.to_string(),
                )),
                duration_ms: Some(duration_ms),
            },
        };

        // Health-only write: `record_health` leaves `updated_at` (the plugin
        // client cache revision) untouched.
        let error_message = (!test_result.ok)
            .then(|| test_result.message.clone())
            .flatten();
        let error_at = (!test_result.ok).then(Utc::now);
        if let Err(error) = self
            .services
            .integrations
            .indexer_proxy_configs
            .record_health(&config.id, test_result.status, error_message, error_at)
            .await
        {
            tracing::warn!(
                proxy_config_id = config.id.as_str(),
                error = %error,
                "failed to persist indexer proxy test result"
            );
        }

        Ok(test_result)
    }
}

fn normalize_indexer_proxy_name(raw: &str) -> AppResult<String> {
    let name = raw.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Validation(
            "indexer proxy name is required".into(),
        ));
    }
    Ok(name)
}

fn normalize_indexer_proxy_base_url(raw: &str) -> AppResult<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(AppError::Validation(
            "indexer proxy base URL is required".into(),
        ));
    }
    let parsed = url::Url::parse(trimmed).map_err(|error| {
        AppError::Validation(format!("invalid indexer proxy base URL: {error}"))
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::Validation(
            "indexer proxy base URL must use http or https".into(),
        ));
    }
    if parsed.host_str().is_none_or(|host| host.trim().is_empty()) {
        return Err(AppError::Validation(
            "indexer proxy base URL must include a host".into(),
        ));
    }
    Ok(trimmed.to_string())
}

fn validate_indexer_proxy_timeout(timeout: u32) -> AppResult<u32> {
    if !(1..=scryer_outbound_http::MAX_INDEXER_PROXY_TIMEOUT_SECONDS).contains(&timeout) {
        return Err(AppError::Validation(format!(
            "indexer proxy timeout must be between 1 and {} seconds",
            scryer_outbound_http::MAX_INDEXER_PROXY_TIMEOUT_SECONDS
        )));
    }
    Ok(timeout)
}

const SOLVER_HEALTH_RESPONSE_MAX_BYTES: usize = 1024 * 1024;

async fn read_solver_health_body_bounded(
    mut response: reqwest::Response,
    provider: scryer_domain::IndexerProxyProviderType,
) -> AppResult<Vec<u8>> {
    let unreadable = || {
        AppError::Repository(
            crate::challenge_solver::solver_error_message(
                provider,
                crate::challenge_solver::SolverErrorKind::Unreadable,
            )
            .into(),
        )
    };
    if response
        .content_length()
        .is_some_and(|length| length > SOLVER_HEALTH_RESPONSE_MAX_BYTES as u64)
    {
        return Err(unreadable());
    }

    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unreadable())? {
        if body.len().saturating_add(chunk.len()) > SOLVER_HEALTH_RESPONSE_MAX_BYTES {
            return Err(unreadable());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn send_solver_probe_request(
    request: reqwest::RequestBuilder,
    provider: scryer_domain::IndexerProxyProviderType,
    deadline: tokio::time::Instant,
) -> AppResult<reqwest::Response> {
    tokio::time::timeout_at(
        deadline,
        scryer_outbound_http::send_reqwest_request_with_cooldown_budget(
            request,
            Some(std::time::Duration::ZERO),
        ),
    )
    .await
    .map_err(|_| {
        AppError::Repository(
            crate::challenge_solver::solver_error_message(
                provider,
                crate::challenge_solver::SolverErrorKind::Timeout,
            )
            .into(),
        )
    })?
    .map_err(|error| {
        let kind = match error {
            scryer_outbound_http::AsyncOutboundHttpError::Request(error) if error.is_timeout() => {
                crate::challenge_solver::SolverErrorKind::Timeout
            }
            scryer_outbound_http::AsyncOutboundHttpError::Request(_) => {
                crate::challenge_solver::SolverErrorKind::Unreachable
            }
            scryer_outbound_http::AsyncOutboundHttpError::CooldownBudgetExceeded { .. } => {
                crate::challenge_solver::SolverErrorKind::Unavailable
            }
        };
        AppError::Repository(crate::challenge_solver::solver_error_message(provider, kind).into())
    })
}

async fn probe_solver_health(config: &scryer_domain::IndexerProxyConfig) -> AppResult<String> {
    let provider_name = crate::challenge_solver::solver_provider_name(config.provider_type);
    let base_url = config.base_url.trim_end_matches('/');
    let health_url = format!("{base_url}/health");
    let request_timeout = scryer_outbound_http::effective_indexer_proxy_request_timeout(
        config.request_timeout_seconds,
    );
    let client = scryer_outbound_http::indexer_proxy_health_reqwest_client(request_timeout)
        .map_err(|_| {
            AppError::Repository(
                crate::challenge_solver::solver_error_message(
                    config.provider_type,
                    crate::challenge_solver::SolverErrorKind::Unreachable,
                )
                .into(),
            )
        })?;
    let health_deadline = tokio::time::Instant::now() + request_timeout;
    let response = send_solver_probe_request(
        client.get(&health_url),
        config.provider_type,
        health_deadline,
    )
    .await?;
    let status = response.status();
    if status.is_success() {
        return Ok(format!(
            "{provider_name} health probe returned HTTP {}",
            status.as_u16()
        ));
    }
    if status != reqwest::StatusCode::NOT_FOUND && status != reqwest::StatusCode::METHOD_NOT_ALLOWED
    {
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            return Err(AppError::Repository(
                crate::challenge_solver::solver_error_message(
                    config.provider_type,
                    crate::challenge_solver::SolverErrorKind::Unavailable,
                )
                .into(),
            ));
        }
        return Err(AppError::Repository(format!(
            "{provider_name} health probe returned HTTP {}",
            status.as_u16()
        )));
    }

    let probe_url = crate::challenge_solver::solver_solve_endpoint(base_url);
    let probe_deadline = tokio::time::Instant::now() + request_timeout;
    let response = send_solver_probe_request(
        client
            .post(&probe_url)
            .json(&crate::challenge_solver::solver_solve_request(
                config.provider_type,
                "https://example.com/",
                config.request_timeout_seconds,
            )),
        config.provider_type,
        probe_deadline,
    )
    .await?;
    let status = response.status();
    if !status.is_success() {
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            return Err(AppError::Repository(
                crate::challenge_solver::solver_error_message(
                    config.provider_type,
                    crate::challenge_solver::SolverErrorKind::Unavailable,
                )
                .into(),
            ));
        }
        return Err(AppError::Repository(format!(
            "{provider_name} v1 probe returned HTTP {}",
            status.as_u16()
        )));
    }
    let body = tokio::time::timeout_at(
        probe_deadline,
        read_solver_health_body_bounded(response, config.provider_type),
    )
    .await
    .map_err(|_| {
        AppError::Repository(
            crate::challenge_solver::solver_error_message(
                config.provider_type,
                crate::challenge_solver::SolverErrorKind::Timeout,
            )
            .into(),
        )
    })??;
    crate::challenge_solver::parse_solver_solution(&body)
        .map_err(|error| AppError::Repository(error.message(config.provider_type).into()))?;
    Ok(format!(
        "{provider_name} v1 probe returned HTTP {}",
        status.as_u16()
    ))
}

#[cfg(test)]
mod indexer_proxy_tests {
    use super::*;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn proxy_timeout_validation_uses_indexer_ceiling() {
        assert_eq!(validate_indexer_proxy_timeout(120).unwrap(), 120);
        assert!(validate_indexer_proxy_timeout(0).is_err());
        assert!(validate_indexer_proxy_timeout(121).is_err());
    }

    fn test_config(
        server: &MockServer,
        provider_type: scryer_domain::IndexerProxyProviderType,
    ) -> scryer_domain::IndexerProxyConfig {
        let now = Utc::now();
        scryer_domain::IndexerProxyConfig {
            id: "proxy-1".into(),
            name: "Solver".into(),
            provider_type,
            protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
            base_url: server.uri(),
            request_timeout_seconds: 60,
            is_enabled: true,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn assert_browser_user_agent(request: &wiremock::Request) {
        assert_eq!(
            request
                .headers
                .get("user-agent")
                .and_then(|value| value.to_str().ok()),
            Some(scryer_outbound_http::INDEXER_PROXY_USER_AGENT)
        );
    }

    #[tokio::test]
    async fn trawl_health_endpoint_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let result = probe_solver_health(&test_config(
            &server,
            scryer_domain::IndexerProxyProviderType::Trawl,
        ))
        .await
        .expect("Trawl health probe should succeed");

        assert_eq!(result, "Trawl health probe returned HTTP 200");
        let requests = server.received_requests().await.expect("recorded requests");
        assert_eq!(requests.len(), 1);
        assert_browser_user_agent(&requests[0]);
    }

    #[tokio::test]
    async fn trawl_health_endpoint_preserves_redirect_behavior() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/ready"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ready"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let result = probe_solver_health(&test_config(
            &server,
            scryer_domain::IndexerProxyProviderType::Trawl,
        ))
        .await
        .expect("redirected Trawl health probe should succeed");

        assert_eq!(result, "Trawl health probe returned HTTP 200");
        let requests = server.received_requests().await.expect("recorded requests");
        let redirected = requests
            .iter()
            .find(|request| request.url.path() == "/ready")
            .expect("redirect target request");
        assert_browser_user_agent(redirected);
    }

    #[tokio::test]
    async fn trawl_health_transport_failure_does_not_fallback_to_v1() {
        use tokio::io::AsyncReadExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener should bind");
        let address = listener
            .local_addr()
            .expect("test listener should have an address");
        let server_task = tokio::spawn(async move {
            let (mut first, _) = listener
                .accept()
                .await
                .expect("health request should connect");
            let mut request = [0_u8; 1024];
            let _ = first.read(&mut request).await;
            drop(first);

            match tokio::time::timeout(std::time::Duration::from_millis(500), listener.accept())
                .await
            {
                Ok(Ok((_second, _))) => 2,
                _ => 1,
            }
        });
        let now = Utc::now();
        let config = scryer_domain::IndexerProxyConfig {
            id: "trawl-transport-failure".into(),
            name: "Trawl".into(),
            provider_type: scryer_domain::IndexerProxyProviderType::Trawl,
            protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
            base_url: format!("http://{address}"),
            request_timeout_seconds: 60,
            is_enabled: true,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
        };

        let error = probe_solver_health(&config)
            .await
            .expect_err("closed health connection should fail immediately");
        let connection_count = server_task.await.expect("test server should join");

        assert!(
            error
                .to_string()
                .contains(crate::challenge_solver::TRAWL_UNREACHABLE_MESSAGE)
        );
        assert_eq!(
            connection_count, 1,
            "the /v1 fallback must not be attempted"
        );
    }

    #[tokio::test]
    async fn trawl_health_fallback_uses_millisecond_v1_timeout() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1"))
            .and(body_json(serde_json::json!({
                "cmd": "request.get",
                "url": "https://example.com/",
                "maxTimeout": 60_000
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ok",
                "solution": {
                    "url": "https://example.com/",
                    "status": 200,
                    "response": "<html></html>",
                    "cookies": [],
                    "userAgent": "Trawl"
                }
            })))
            .mount(&server)
            .await;

        let result = probe_solver_health(&test_config(
            &server,
            scryer_domain::IndexerProxyProviderType::Trawl,
        ))
        .await
        .expect("Trawl v1 fallback should succeed");

        assert_eq!(result, "Trawl v1 probe returned HTTP 200");
        let requests = server.received_requests().await.expect("recorded requests");
        let fallback = requests
            .iter()
            .find(|request| request.url.path() == "/v1")
            .expect("v1 fallback request");
        assert_browser_user_agent(fallback);
    }

    #[tokio::test]
    async fn trawl_health_fallback_rejects_malformed_solver_output() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(405))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let error = probe_solver_health(&test_config(
            &server,
            scryer_domain::IndexerProxyProviderType::Trawl,
        ))
        .await
        .expect_err("malformed Trawl output should fail");

        assert!(
            error
                .to_string()
                .contains(crate::challenge_solver::TRAWL_MALFORMED_MESSAGE)
        );
    }

    #[tokio::test]
    async fn trawl_health_fallback_rejects_error_envelope_with_solution() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "error",
                "message": "Browser pool initializing, retry in a few seconds",
                "solution": {
                    "url": "https://example.com/",
                    "status": 0,
                    "headers": {},
                    "response": "",
                    "cookies": [],
                    "userAgent": ""
                }
            })))
            .mount(&server)
            .await;

        let error = probe_solver_health(&test_config(
            &server,
            scryer_domain::IndexerProxyProviderType::Trawl,
        ))
        .await
        .expect_err("Trawl error envelopes must fail health checks");

        assert!(
            error
                .to_string()
                .contains(crate::challenge_solver::TRAWL_UNAVAILABLE_MESSAGE)
        );
    }

    #[tokio::test]
    async fn trawl_health_fallback_rejects_oversized_solver_output() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![
                b'x';
                SOLVER_HEALTH_RESPONSE_MAX_BYTES
                    + 1
            ]))
            .mount(&server)
            .await;

        let error = probe_solver_health(&test_config(
            &server,
            scryer_domain::IndexerProxyProviderType::Trawl,
        ))
        .await
        .expect_err("oversized Trawl output must fail health checks");

        assert!(
            error
                .to_string()
                .contains(crate::challenge_solver::TRAWL_UNREADABLE_MESSAGE)
        );
    }

    #[tokio::test]
    async fn trawl_health_rate_limit_is_classified_as_solver_unavailable() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;

        let error = probe_solver_health(&test_config(
            &server,
            scryer_domain::IndexerProxyProviderType::Trawl,
        ))
        .await
        .expect_err("rate-limited Trawl health should fail");

        assert!(
            error
                .to_string()
                .contains(crate::challenge_solver::TRAWL_UNAVAILABLE_MESSAGE)
        );
    }
}
