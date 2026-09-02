impl AppUseCase {
    pub async fn list_proxy_configs(
        &self,
        actor: &User,
    ) -> AppResult<Vec<scryer_domain::ProxyConfig>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.services.integrations.proxy_configs.list(None).await
    }

    pub async fn create_proxy_config(
        &self,
        actor: &User,
        input: NewProxyConfig,
    ) -> AppResult<scryer_domain::ProxyConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let provider_type = input.provider_type;
        let name = normalize_proxy_name(&input.name)?;
        let endpoint = normalize_proxy_endpoint(provider_type, &input.base_url)?;
        let request_timeout_seconds =
            validate_proxy_timeout(input.request_timeout_seconds.unwrap_or(60))?;
        let protocol = resolve_new_proxy_protocol(provider_type, input.protocol)?;
        let username = normalize_proxy_credential(input.username);
        let password = normalize_proxy_credential(input.password);
        validate_proxy_credentials(provider_type, username.as_deref(), password.as_deref())?;
        let private_key = normalize_proxy_private_key(input.private_key);
        let private_key_passphrase = normalize_proxy_credential(input.private_key_passphrase);
        validate_tunnel_auth(
            provider_type,
            username.as_deref(),
            password.as_deref(),
            private_key.as_deref(),
            private_key_passphrase.as_deref(),
        )?;
        let remote_dns =
            resolve_remote_dns(provider_type, input.remote_dns, endpoint.scheme_remote_dns)?;
        let now = Utc::now();
        let config = scryer_domain::ProxyConfig {
            id: Id::new().0,
            name,
            provider_type,
            protocol,
            base_url: endpoint.base_url,
            request_timeout_seconds,
            is_enabled: input.is_enabled,
            username_encrypted: username,
            password_encrypted: password,
            remote_dns,
            last_health_status: Some(scryer_domain::ProxyHealthStatus::Unknown),
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
            // A host key is pinned on the first successful connect (WP6), never
            // supplied by the operator, so a new row always starts unpinned.
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            private_key_encrypted: private_key,
            private_key_passphrase_encrypted: private_key_passphrase,
        };
        self.services
            .integrations
            .proxy_configs
            .create(config)
            .await
    }

    pub async fn update_proxy_config(
        &self,
        actor: &User,
        update: ProxyConfigUpdate,
    ) -> AppResult<scryer_domain::ProxyConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let id = update.id.trim();
        if id.is_empty() {
            return Err(AppError::Validation("proxy config id is required".into()));
        }
        if !update.has_changes() {
            return Err(AppError::Validation(
                "at least one proxy field must be provided".into(),
            ));
        }

        let mut config = self
            .services
            .integrations
            .proxy_configs
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("proxy config '{id}' not found")))?;
        if let Some(name) = update.name {
            config.name = normalize_proxy_name(&name)?;
        }
        // Only a base URL supplied by *this* patch can imply remote DNS; the
        // stored flag is otherwise left to `update.remote_dns` alone.
        let mut scheme_remote_dns = None;
        if let Some(base_url) = update.base_url {
            let endpoint = normalize_proxy_endpoint(config.provider_type, &base_url)?;
            config.base_url = endpoint.base_url;
            scheme_remote_dns = endpoint.scheme_remote_dns;
        }
        if let Some(timeout) = update.request_timeout_seconds {
            config.request_timeout_seconds = validate_proxy_timeout(timeout)?;
        }
        // Credentials are write-only: an omitted field keeps the stored secret,
        // an explicit null clears it.
        if let Some(username) = update.username {
            config.username_encrypted = normalize_proxy_credential(username);
        }
        if let Some(password) = update.password {
            config.password_encrypted = normalize_proxy_credential(password);
        }
        if let Some(private_key) = update.private_key {
            config.private_key_encrypted = normalize_proxy_private_key(private_key);
        }
        if let Some(passphrase) = update.private_key_passphrase {
            config.private_key_passphrase_encrypted = normalize_proxy_credential(passphrase);
        }
        validate_proxy_credentials(
            config.provider_type,
            config.username_encrypted.as_deref(),
            config.password_encrypted.as_deref(),
        )?;
        validate_tunnel_auth(
            config.provider_type,
            config.username_encrypted.as_deref(),
            config.password_encrypted.as_deref(),
            config.private_key_encrypted.as_deref(),
            config.private_key_passphrase_encrypted.as_deref(),
        )?;
        if update.remote_dns.is_some() || scheme_remote_dns.is_some() {
            config.remote_dns =
                resolve_remote_dns(config.provider_type, update.remote_dns, scheme_remote_dns)?;
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
                    indexer.is_enabled && indexer.proxy_config_id.as_deref() == Some(id)
                })
                .count();
            if assigned_count > 0 {
                return Err(AppError::Validation(format!(
                    "proxy config is assigned to {assigned_count} enabled indexer(s)"
                )));
            }
            // Download clients are consumers on exactly the same terms, so the
            // same guard applies: disabling a proxy out from under live traffic
            // would silently route it somewhere the operator did not choose.
            let assigned_clients = self
                .services
                .integrations
                .download_client_configs
                .list(None)
                .await?
                .into_iter()
                .filter(|client| client.is_enabled && client.proxy_config_id.as_deref() == Some(id))
                .count();
            if assigned_clients > 0 {
                return Err(AppError::Validation(format!(
                    "proxy config is assigned to {assigned_clients} enabled download client(s)"
                )));
            }
        }
        if let Some(is_enabled) = update.is_enabled {
            config.is_enabled = is_enabled;
        }
        config.updated_at = Utc::now();

        self.services
            .integrations
            .proxy_configs
            .update(config)
            .await
    }

    pub async fn delete_proxy_config(&self, actor: &User, id: &str) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let id = id.trim();
        if id.is_empty() {
            return Err(AppError::Validation("proxy config id is required".into()));
        }
        let assigned = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?
            .into_iter()
            .any(|indexer| indexer.proxy_config_id.as_deref() == Some(id));
        if assigned {
            return Err(AppError::Validation(
                "proxy config is assigned to one or more indexers".into(),
            ));
        }
        // Same rule for the other consumer family. Deletion refuses rather than
        // clearing assignments, which is how the indexer side has always
        // behaved: silently unproxying live traffic is the worse outcome.
        let assigned_clients = self
            .services
            .integrations
            .download_client_configs
            .list(None)
            .await?
            .into_iter()
            .any(|client| client.proxy_config_id.as_deref() == Some(id));
        if assigned_clients {
            return Err(AppError::Validation(
                "proxy config is assigned to one or more download clients".into(),
            ));
        }
        self.services.integrations.proxy_configs.delete(id).await
    }

    /// Forget the pinned tunnel host key so the next connect trusts the server
    /// afresh.
    ///
    /// Trust-on-first-use is only usable if there is a way back: a legitimate
    /// server rekey would otherwise leave the operator with a proxy that hard-
    /// fails forever. Clearing is deliberately explicit and operator-driven,
    /// never automatic on a mismatch.
    pub async fn reset_proxy_host_key(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<scryer_domain::ProxyConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let id = id.trim();
        if id.is_empty() {
            return Err(AppError::Validation("proxy config id is required".into()));
        }
        let config = self
            .services
            .integrations
            .proxy_configs
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("proxy config '{id}' not found")))?;
        if !config.is_tunnel() {
            return Err(AppError::Validation(
                "only tunnel proxies pin a host key".into(),
            ));
        }
        self.services
            .integrations
            .proxy_configs
            .clear_host_key(&config.id)
            .await?;
        self.services
            .integrations
            .proxy_configs
            .get_by_id(&config.id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("proxy config '{id}' not found")))
    }

    pub async fn test_proxy_config(&self, actor: &User, id: &str) -> AppResult<ProxyTestResult> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let id = id.trim();
        if id.is_empty() {
            return Err(AppError::Validation("proxy config id is required".into()));
        }
        let config = self
            .services
            .integrations
            .proxy_configs
            .get_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("proxy config '{id}' not found")))?;

        let started = std::time::Instant::now();
        let result = match config.kind() {
            scryer_domain::ProxyKind::ChallengeSolver => probe_solver_health(&config).await,
            scryer_domain::ProxyKind::Transport => {
                let destination = self.transport_proxy_probe_destination(&config).await;
                probe_transport_proxy_health(&config, destination.as_deref()).await
            }
            // No fake pass. There is no tunnel to probe until the engine that
            // builds one exists; WP6 replaces this arm with a handshake plus
            // the same assigned-destination fetch the transport arm does.
            scryer_domain::ProxyKind::Tunnel => Err(AppError::Validation(
                crate::tunnel_proxy::TUNNEL_PROBE_UNAVAILABLE_MESSAGE.to_string(),
            )),
        };
        let duration_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        let test_result = match result {
            Ok(message) => ProxyTestResult {
                ok: true,
                status: scryer_domain::ProxyHealthStatus::Healthy,
                message: Some(message),
                duration_ms: Some(duration_ms),
            },
            Err(error) => ProxyTestResult {
                ok: false,
                status: scryer_domain::ProxyHealthStatus::Unhealthy,
                message: Some(crate::challenge_solver::sanitize_proxy_error(
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
            .proxy_configs
            .record_health(&config.id, test_result.status, error_message, error_at)
            .await
        {
            tracing::warn!(
                proxy_config_id = config.id.as_str(),
                error = %error,
                "failed to persist proxy test result"
            );
        }

        Ok(test_result)
    }

    /// The URL a transport-proxy test should fetch *through* the proxy.
    ///
    /// The operator's own assigned indexer is the only destination we know is
    /// meant to be reachable this way, so the probe borrows it. With nothing
    /// assigned there is no honest destination to pick and the probe falls
    /// back to checking the proxy endpoint alone.
    async fn transport_proxy_probe_destination(
        &self,
        config: &scryer_domain::ProxyConfig,
    ) -> Option<String> {
        let indexers = match self.services.integrations.indexer_configs.list(None).await {
            Ok(indexers) => indexers,
            Err(error) => {
                tracing::warn!(
                    proxy_config_id = config.id.as_str(),
                    error = %error,
                    "could not list indexers for the transport proxy probe"
                );
                return None;
            }
        };
        let mut assigned: Vec<_> = indexers
            .into_iter()
            .filter(|indexer| {
                indexer.proxy_config_id.as_deref() == Some(config.id.as_str())
                    && !indexer.base_url.trim().is_empty()
            })
            .collect();
        // Prefer an enabled indexer, then keep the repository's own ordering so
        // repeated tests probe the same destination.
        assigned.sort_by_key(|indexer| !indexer.is_enabled);
        assigned
            .into_iter()
            .next()
            .map(|indexer| indexer.base_url.trim().to_string())
    }
}

fn normalize_proxy_name(raw: &str) -> AppResult<String> {
    let name = raw.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Validation("proxy name is required".into()));
    }
    Ok(name)
}

fn normalize_proxy_base_url(raw: &str) -> AppResult<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(AppError::Validation("proxy base URL is required".into()));
    }
    let parsed = url::Url::parse(trimmed)
        .map_err(|error| AppError::Validation(format!("invalid proxy base URL: {error}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::Validation(
            "proxy base URL must use http or https".into(),
        ));
    }
    if parsed.host_str().is_none_or(|host| host.trim().is_empty()) {
        return Err(AppError::Validation(
            "proxy base URL must include a host".into(),
        ));
    }
    Ok(trimmed.to_string())
}

/// A validated proxy endpoint plus whatever its scheme said about remote DNS.
#[derive(Debug)]
struct NormalizedProxyEndpoint {
    base_url: String,
    /// `Some(true)` when the operator wrote `socks5h://` or `socks4a://`.
    /// `None` when the scheme says nothing about DNS.
    scheme_remote_dns: Option<bool>,
}

/// Validate a proxy endpoint against what its provider actually is.
///
/// Challenge solvers keep the original http/https rule untouched. Transport
/// and tunnel providers additionally have to match their own scheme, because a
/// `socks5` row whose URL says `http` would be silently unusable at egress
/// time.
fn normalize_proxy_endpoint(
    provider_type: scryer_domain::ProxyProviderType,
    raw: &str,
) -> AppResult<NormalizedProxyEndpoint> {
    use scryer_domain::ProxyProviderType as Provider;

    if provider_type.is_challenge_solver() {
        return Ok(NormalizedProxyEndpoint {
            base_url: normalize_proxy_base_url(raw)?,
            scheme_remote_dns: None,
        });
    }

    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(AppError::Validation("proxy base URL is required".into()));
    }
    let parsed = url::Url::parse(trimmed)
        .map_err(|error| AppError::Validation(format!("invalid proxy base URL: {error}")))?;
    if parsed.host_str().is_none_or(|host| host.trim().is_empty()) {
        return Err(AppError::Validation(
            "proxy base URL must include a host".into(),
        ));
    }
    // Credentials belong in the username/password fields, which are encrypted
    // at rest. The base URL is stored in the clear.
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppError::Validation(
            "proxy base URL must not embed credentials; use the username and password fields"
                .into(),
        ));
    }

    match (provider_type, parsed.scheme()) {
        (Provider::Http, "http" | "https") => Ok(NormalizedProxyEndpoint {
            base_url: trimmed.to_string(),
            scheme_remote_dns: None,
        }),
        // An SSH endpoint is a host and a port and nothing else: there is no
        // resource to address, so a path is a sign the operator pasted the
        // wrong thing.
        (Provider::SshTunnel, "ssh") => {
            if !matches!(parsed.path(), "" | "/") {
                return Err(AppError::Validation(
                    "SSH tunnel base URL must be ssh://host[:port] with no path".into(),
                ));
            }
            if parsed.query().is_some() || parsed.fragment().is_some() {
                return Err(AppError::Validation(
                    "SSH tunnel base URL must be ssh://host[:port] with no query or fragment"
                        .into(),
                ));
            }
            Ok(NormalizedProxyEndpoint {
                base_url: trimmed.to_string(),
                scheme_remote_dns: None,
            })
        }
        (Provider::Socks4, "socks4") | (Provider::Socks5, "socks5") => {
            Ok(NormalizedProxyEndpoint {
                base_url: trimmed.to_string(),
                scheme_remote_dns: None,
            })
        }
        // `socks5h` / `socks4a` are the same proxy with proxy-side name
        // resolution. Store the canonical local-DNS URL and carry the
        // difference in `remote_dns` so there is one place that answers "where
        // is DNS resolved?".
        (Provider::Socks4, "socks4a") | (Provider::Socks5, "socks5h") => {
            let canonical_scheme = match provider_type {
                Provider::Socks4 => "socks4",
                _ => "socks5",
            };
            let mut canonical = parsed.clone();
            canonical.set_scheme(canonical_scheme).map_err(|_| {
                AppError::Validation(format!("invalid {} proxy base URL", parsed.scheme()))
            })?;
            Ok(NormalizedProxyEndpoint {
                base_url: canonical.as_str().trim_end_matches('/').to_string(),
                scheme_remote_dns: Some(true),
            })
        }
        (Provider::Http, _) => Err(AppError::Validation(
            "HTTP proxy base URL must use http or https".into(),
        )),
        (Provider::Socks4, _) => Err(AppError::Validation(
            "SOCKS4 proxy base URL must use socks4 or socks4a".into(),
        )),
        (Provider::Socks5, _) => Err(AppError::Validation(
            "SOCKS5 proxy base URL must use socks5 or socks5h".into(),
        )),
        (Provider::SshTunnel, _) => Err(AppError::Validation(
            "SSH tunnel base URL must use ssh".into(),
        )),
        (Provider::Byparr | Provider::Trawl, _) => unreachable!("handled by the solver branch"),
    }
}

/// Solver providers take the one protocol Scryer speaks; transport providers
/// take none at all, and being handed one means the caller is confused about
/// what it is configuring.
fn resolve_new_proxy_protocol(
    provider_type: scryer_domain::ProxyProviderType,
    requested: Option<scryer_domain::ChallengeSolverProtocol>,
) -> AppResult<Option<scryer_domain::ChallengeSolverProtocol>> {
    match provider_type.kind() {
        scryer_domain::ProxyKind::ChallengeSolver => Ok(Some(
            requested.unwrap_or(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1),
        )),
        scryer_domain::ProxyKind::Transport | scryer_domain::ProxyKind::Tunnel
            if requested.is_some() =>
        {
            Err(AppError::Validation(format!(
                "{} proxies do not use a challenge-solver protocol",
                provider_type.as_str()
            )))
        }
        scryer_domain::ProxyKind::Transport | scryer_domain::ProxyKind::Tunnel => Ok(None),
    }
}

fn normalize_proxy_credential(raw: Option<String>) -> Option<String> {
    raw.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn validate_proxy_credentials(
    provider_type: scryer_domain::ProxyProviderType,
    username: Option<&str>,
    password: Option<&str>,
) -> AppResult<()> {
    if provider_type.is_challenge_solver() && (username.is_some() || password.is_some()) {
        return Err(AppError::Validation(
            "challenge-solver proxies do not accept a username or password".into(),
        ));
    }
    // SOCKS4 authentication is not wired in our HTTP client (reqwest builds its
    // SOCKS4 connector without `with_auth`, so a username would be silently
    // dropped on the wire). Rejecting is honest; accepting would promise an
    // authenticated hop that never happens.
    if provider_type == scryer_domain::ProxyProviderType::Socks4
        && (username.is_some() || password.is_some())
    {
        return Err(AppError::Validation(
            "SOCKS4 proxies do not carry credentials; use SOCKS5 for an authenticated proxy".into(),
        ));
    }
    if password.is_some() && username.is_none() {
        return Err(AppError::Validation(
            "proxy password requires a username".into(),
        ));
    }
    Ok(())
}

/// A pasted private key is normalized like any other secret, but keeps its
/// internal newlines: a PEM body is line-structured and trimming only the ends
/// is what makes a copy-pasted block usable.
fn normalize_proxy_private_key(raw: Option<String>) -> Option<String> {
    raw.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Structural PEM check only.
///
/// Real key parsing belongs to the tunnel engine (WP6, russh-keys); rejecting
/// something that is plainly not a PEM block here just keeps an obvious paste
/// mistake from being stored encrypted and failing much later with a worse
/// message.
fn validate_private_key_pem(pem: &str) -> AppResult<()> {
    let trimmed = pem.trim();
    let malformed = || {
        AppError::Validation(
            "private key must be a PEM block beginning with -----BEGIN and ending with -----"
                .to_string(),
        )
    };
    if !trimmed.starts_with("-----BEGIN") {
        return Err(malformed());
    }
    let last_line = trimmed.lines().next_back().unwrap_or_default().trim();
    if !(last_line.starts_with("-----END") && last_line.ends_with("-----")) {
        return Err(malformed());
    }
    Ok(())
}

/// Authentication rules for the tunnel family, and the rejection that keeps
/// key material off every other family.
fn validate_tunnel_auth(
    provider_type: scryer_domain::ProxyProviderType,
    username: Option<&str>,
    password: Option<&str>,
    private_key: Option<&str>,
    private_key_passphrase: Option<&str>,
) -> AppResult<()> {
    if !provider_type.is_tunnel() {
        if private_key.is_some() {
            return Err(AppError::Validation(
                "only SSH tunnels take a private key".into(),
            ));
        }
        if private_key_passphrase.is_some() {
            return Err(AppError::Validation(
                "only SSH tunnels take a private key passphrase".into(),
            ));
        }
        return Ok(());
    }

    if username.is_none() {
        return Err(AppError::Validation(
            "SSH tunnels require a username".into(),
        ));
    }
    if password.is_none() && private_key.is_none() {
        return Err(AppError::Validation(
            "SSH tunnels require a password or a private key".into(),
        ));
    }
    if private_key_passphrase.is_some() && private_key.is_none() {
        return Err(AppError::Validation(
            "a private key passphrase requires a private key".into(),
        ));
    }
    if let Some(private_key) = private_key {
        validate_private_key_pem(private_key)?;
    }
    Ok(())
}

/// Resolve the stored remote-DNS flag from what the operator asked for and
/// what the URL scheme implied.
fn resolve_remote_dns(
    provider_type: scryer_domain::ProxyProviderType,
    requested: Option<bool>,
    scheme_remote_dns: Option<bool>,
) -> AppResult<bool> {
    if scheme_remote_dns == Some(true) && requested == Some(false) {
        return Err(AppError::Validation(
            "socks5h:// and socks4a:// resolve names at the proxy; use socks5:// or socks4:// to resolve them locally"
                .into(),
        ));
    }
    let resolved = requested.or(scheme_remote_dns).unwrap_or(false);
    // SOCKS is the only family where Scryer chooses: reqwest expresses
    // proxy-side resolution as `socks5h` / `socks4a`. An HTTP CONNECT proxy
    // always receives the destination as a name and resolves it itself, so
    // there is no flag to set, and a solver fetches the page entirely on its
    // own side.
    if resolved
        && !matches!(
            provider_type,
            scryer_domain::ProxyProviderType::Socks4 | scryer_domain::ProxyProviderType::Socks5
        )
    {
        return Err(AppError::Validation(
            "remote DNS applies only to SOCKS proxies".into(),
        ));
    }
    Ok(resolved)
}

fn validate_proxy_timeout(timeout: u32) -> AppResult<u32> {
    if !(1..=scryer_outbound_http::MAX_PROXY_TIMEOUT_SECONDS).contains(&timeout) {
        return Err(AppError::Validation(format!(
            "proxy timeout must be between 1 and {} seconds",
            scryer_outbound_http::MAX_PROXY_TIMEOUT_SECONDS
        )));
    }
    Ok(timeout)
}

const SOLVER_HEALTH_RESPONSE_MAX_BYTES: usize = 1024 * 1024;

async fn read_solver_health_body_bounded(
    mut response: reqwest::Response,
    provider: scryer_domain::ProxyProviderType,
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
    provider: scryer_domain::ProxyProviderType,
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

async fn probe_solver_health(config: &scryer_domain::ProxyConfig) -> AppResult<String> {
    let provider_name = crate::challenge_solver::solver_provider_name(config.provider_type);
    let base_url = config.base_url.trim_end_matches('/');
    let health_url = format!("{base_url}/health");
    let request_timeout =
        scryer_outbound_http::effective_proxy_request_timeout(config.request_timeout_seconds);
    let client =
        scryer_outbound_http::proxy_health_reqwest_client(request_timeout).map_err(|_| {
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

/// Marker phrases that let an operator (and the UI) tell "we could not even
/// reach the proxy" apart from "the proxy is there, the far end is not".
pub const TRANSPORT_PROXY_UNREACHABLE_MESSAGE: &str = "transport proxy is unreachable";
pub const TRANSPORT_PROXY_TIMEOUT_MESSAGE: &str = "transport proxy did not answer in time";
pub const TRANSPORT_PROXY_DOWNSTREAM_MESSAGE: &str =
    "transport proxy answered, but the request through it failed";

fn transport_proxy_name(provider_type: scryer_domain::ProxyProviderType) -> &'static str {
    match provider_type {
        scryer_domain::ProxyProviderType::Http => "HTTP proxy",
        scryer_domain::ProxyProviderType::Socks4 => "SOCKS4 proxy",
        scryer_domain::ProxyProviderType::Socks5 => "SOCKS5 proxy",
        scryer_domain::ProxyProviderType::SshTunnel => "SSH tunnel",
        scryer_domain::ProxyProviderType::Byparr | scryer_domain::ProxyProviderType::Trawl => {
            "challenge solver"
        }
    }
}

/// Default port for a proxy family whose scheme the URL crate has no default
/// for: SOCKS listens on 1080, SSH on 22.
fn proxy_default_port(provider_type: scryer_domain::ProxyProviderType) -> Option<u16> {
    match provider_type {
        scryer_domain::ProxyProviderType::Socks4 | scryer_domain::ProxyProviderType::Socks5 => {
            Some(1080)
        }
        scryer_domain::ProxyProviderType::SshTunnel => Some(22),
        // http/https already have a known default, and a solver is addressed
        // as an ordinary URL.
        scryer_domain::ProxyProviderType::Http
        | scryer_domain::ProxyProviderType::Byparr
        | scryer_domain::ProxyProviderType::Trawl => None,
    }
}

/// Split a stored transport-proxy base URL into the host and port to dial.
fn transport_proxy_endpoint(config: &scryer_domain::ProxyConfig) -> AppResult<(String, u16)> {
    let parsed = url::Url::parse(config.base_url.trim())
        .map_err(|error| AppError::Validation(format!("invalid proxy base URL: {error}")))?;
    let host = parsed
        .host_str()
        .map(str::to_string)
        .filter(|host| !host.trim().is_empty())
        .ok_or_else(|| AppError::Validation("proxy base URL must include a host".to_string()))?;
    // SOCKS and ssh URLs carry no scheme-implied default in the URL crate, so
    // supply the same defaults our clients would.
    let port = parsed
        .port_or_known_default()
        .or_else(|| proxy_default_port(config.provider_type))
        .ok_or_else(|| AppError::Validation("proxy base URL must include a port".to_string()))?;
    Ok((host, port))
}

/// Confirm the proxy endpoint itself is listening.
///
/// This is a TCP connect, not a protocol handshake: it proves the operator's
/// host and port are right and something is accepting there, which is exactly
/// the failure this separates out from "the request through the proxy failed".
async fn probe_transport_proxy_reachable(
    config: &scryer_domain::ProxyConfig,
    timeout: std::time::Duration,
) -> AppResult<(String, u16)> {
    let (host, port) = transport_proxy_endpoint(config)?;
    let stream = tokio::time::timeout(
        timeout,
        tokio::net::TcpStream::connect((host.as_str(), port)),
    )
    .await
    .map_err(|_| {
        AppError::Repository(format!("{TRANSPORT_PROXY_TIMEOUT_MESSAGE} ({host}:{port})"))
    })?
    .map_err(|error| {
        AppError::Repository(format!(
            "{TRANSPORT_PROXY_UNREACHABLE_MESSAGE} ({host}:{port}): {error}"
        ))
    })?;
    drop(stream);
    Ok((host, port))
}

/// Health-check a transport proxy.
///
/// Reachability of the proxy endpoint is always checked first so a failure can
/// be attributed. When the operator has an indexer assigned to this proxy, its
/// base URL is then fetched *through* the proxy; any HTTP answer proves the
/// tunnel carries traffic, so the status code is reported rather than judged —
/// this is a test of the proxy, not of the indexer's credentials.
async fn probe_transport_proxy_health(
    config: &scryer_domain::ProxyConfig,
    destination: Option<&str>,
) -> AppResult<String> {
    let proxy_name = transport_proxy_name(config.provider_type);
    let request_timeout =
        scryer_outbound_http::effective_proxy_request_timeout(config.request_timeout_seconds);
    let (host, port) = probe_transport_proxy_reachable(config, request_timeout).await?;

    let Some(destination) = destination else {
        return Ok(format!(
            "{proxy_name} accepted a connection on {host}:{port}; assign an indexer to this proxy to test a request through it"
        ));
    };

    let client = scryer_outbound_http::transport_proxy_reqwest_client(
        &effective_transport_proxy_url(config),
        transport_proxy_credentials(config),
        request_timeout,
    )
    .map_err(|error| {
        AppError::Repository(format!(
            "{TRANSPORT_PROXY_UNREACHABLE_MESSAGE} ({host}:{port}): {error}"
        ))
    })?;
    let response = tokio::time::timeout(request_timeout, client.get(destination).send())
        .await
        .map_err(|_| {
            AppError::Repository(format!(
                "{TRANSPORT_PROXY_DOWNSTREAM_MESSAGE}: {destination} timed out"
            ))
        })?
        .map_err(|error| {
            AppError::Repository(format!(
                "{TRANSPORT_PROXY_DOWNSTREAM_MESSAGE}: {destination}: {error}"
            ))
        })?;
    Ok(format!(
        "{proxy_name} carried a request to {destination} (HTTP {})",
        response.status().as_u16()
    ))
}

use crate::transport_proxy::{
    transport_proxy_credentials, transport_proxy_egress_url as effective_transport_proxy_url,
};

#[cfg(test)]
mod proxy_tests {
    use super::*;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use scryer_domain::ProxyProviderType as Provider;

    #[test]
    fn proxy_timeout_validation_uses_indexer_ceiling() {
        assert_eq!(validate_proxy_timeout(120).unwrap(), 120);
        assert!(validate_proxy_timeout(0).is_err());
        assert!(validate_proxy_timeout(121).is_err());
    }

    #[test]
    fn solver_endpoints_keep_the_original_http_rule() {
        let endpoint = normalize_proxy_endpoint(Provider::Trawl, " http://solver:8191/ ")
            .expect("solver URLs are unchanged");
        assert_eq!(endpoint.base_url, "http://solver:8191");
        assert_eq!(endpoint.scheme_remote_dns, None);
        assert!(normalize_proxy_endpoint(Provider::Byparr, "socks5://solver:1080").is_err());
    }

    #[test]
    fn transport_endpoints_must_match_their_provider_scheme() {
        assert_eq!(
            normalize_proxy_endpoint(Provider::Http, "http://gateway:3128")
                .expect("http proxy")
                .base_url,
            "http://gateway:3128"
        );
        assert_eq!(
            normalize_proxy_endpoint(Provider::Socks5, "socks5://gateway:1080")
                .expect("socks5 proxy")
                .base_url,
            "socks5://gateway:1080"
        );
        assert!(normalize_proxy_endpoint(Provider::Http, "socks5://gateway:1080").is_err());
        assert!(normalize_proxy_endpoint(Provider::Socks5, "http://gateway:3128").is_err());
    }

    #[test]
    fn socks5h_is_stored_as_socks5_plus_remote_dns() {
        let endpoint = normalize_proxy_endpoint(Provider::Socks5, "socks5h://gateway:1080")
            .expect("socks5h is accepted");
        assert_eq!(endpoint.base_url, "socks5://gateway:1080");
        assert_eq!(endpoint.scheme_remote_dns, Some(true));
        assert!(resolve_remote_dns(Provider::Socks5, None, Some(true)).expect("implied"));
        // Asking for local DNS while writing socks5h:// is a contradiction.
        assert!(resolve_remote_dns(Provider::Socks5, Some(false), Some(true)).is_err());
    }

    #[test]
    fn transport_endpoints_reject_credentials_in_the_url() {
        let error = normalize_proxy_endpoint(Provider::Socks5, "socks5://u:p@gateway:1080")
            .expect_err("credentials belong in the encrypted fields");
        assert!(error.to_string().contains("must not embed credentials"));
    }

    #[test]
    fn remote_dns_is_only_meaningful_for_socks() {
        assert!(!resolve_remote_dns(Provider::Http, Some(false), None).expect("off is fine"));
        // An HTTP CONNECT proxy always forwards a hostname; there is no flag to
        // set, so asking for one is a configuration error rather than a no-op.
        assert!(resolve_remote_dns(Provider::Http, Some(true), None).is_err());
        assert!(resolve_remote_dns(Provider::Trawl, Some(true), None).is_err());
        assert!(resolve_remote_dns(Provider::Socks5, Some(true), None).expect("socks5 allows it"));
        // reqwest speaks socks4a, so SOCKS4 gets proxy-side DNS as well.
        assert!(resolve_remote_dns(Provider::Socks4, Some(true), None).expect("socks4a"));
    }

    #[test]
    fn socks4_endpoints_round_trip_like_socks5() {
        assert_eq!(
            normalize_proxy_endpoint(Provider::Socks4, "socks4://gateway:1080")
                .expect("socks4 proxy")
                .base_url,
            "socks4://gateway:1080"
        );
        let endpoint = normalize_proxy_endpoint(Provider::Socks4, "socks4a://gateway:1080")
            .expect("socks4a is accepted");
        assert_eq!(endpoint.base_url, "socks4://gateway:1080");
        assert_eq!(endpoint.scheme_remote_dns, Some(true));
        assert!(normalize_proxy_endpoint(Provider::Socks4, "socks5://gateway:1080").is_err());
        assert!(normalize_proxy_endpoint(Provider::Socks5, "socks4://gateway:1080").is_err());
    }

    /// reqwest builds its SOCKS4 connector without `with_auth`, so a username
    /// would be dropped on the wire. Rejecting is honest; silently accepting
    /// would promise an authenticated hop that never happens.
    #[test]
    fn socks4_rejects_credentials_it_cannot_send() {
        assert!(validate_proxy_credentials(Provider::Socks4, None, None).is_ok());
        let error = validate_proxy_credentials(Provider::Socks4, Some("operator"), None)
            .expect_err("socks4 cannot carry a username");
        assert!(error.to_string().contains("SOCKS4"), "{error}");
        assert!(
            validate_proxy_credentials(Provider::Socks5, Some("operator"), Some("s3cret")).is_ok()
        );
        assert!(
            validate_proxy_credentials(Provider::Http, Some("operator"), Some("s3cret")).is_ok()
        );
    }

    #[test]
    fn ssh_tunnel_endpoints_are_host_and_port_only() {
        assert_eq!(
            normalize_proxy_endpoint(Provider::SshTunnel, "ssh://seedbox.test:2222")
                .expect("ssh endpoint")
                .base_url,
            "ssh://seedbox.test:2222"
        );
        // The port is optional in the URL; 22 is supplied when the endpoint is
        // dialled rather than being written into the stored value.
        assert_eq!(
            normalize_proxy_endpoint(Provider::SshTunnel, "ssh://seedbox.test")
                .expect("default port")
                .base_url,
            "ssh://seedbox.test"
        );
        assert_eq!(proxy_default_port(Provider::SshTunnel), Some(22));
        // Everything else about the URL is a sign of a bad paste.
        assert!(normalize_proxy_endpoint(Provider::SshTunnel, "ssh://seedbox.test/path").is_err());
        assert!(normalize_proxy_endpoint(Provider::SshTunnel, "ssh://seedbox.test?x=1").is_err());
        assert!(
            normalize_proxy_endpoint(Provider::SshTunnel, "ssh://user:pw@seedbox.test").is_err()
        );
        assert!(normalize_proxy_endpoint(Provider::SshTunnel, "socks5://seedbox.test").is_err());
        assert!(normalize_proxy_endpoint(Provider::Socks5, "ssh://seedbox.test").is_err());
        // A trailing slash is the one path form that survives, because it names
        // no resource.
        assert_eq!(
            normalize_proxy_endpoint(Provider::SshTunnel, "ssh://seedbox.test:22/")
                .expect("trailing slash")
                .base_url,
            "ssh://seedbox.test:22"
        );
    }

    #[test]
    fn ssh_tunnels_require_a_username_and_one_form_of_authentication() {
        const PEM: &str =
            "-----BEGIN OPENSSH PRIVATE KEY-----\nbody\n-----END OPENSSH PRIVATE KEY-----";

        // Username is not optional: there is no "current user" to fall back to.
        assert!(
            validate_tunnel_auth(Provider::SshTunnel, None, Some("s3cret"), None, None).is_err()
        );
        // Neither a password nor a key means nothing to authenticate with.
        assert!(
            validate_tunnel_auth(Provider::SshTunnel, Some("operator"), None, None, None).is_err()
        );
        // Either alone is enough, and both together is allowed (WP6 prefers the
        // key).
        assert!(
            validate_tunnel_auth(
                Provider::SshTunnel,
                Some("operator"),
                Some("s3cret"),
                None,
                None
            )
            .is_ok()
        );
        assert!(
            validate_tunnel_auth(Provider::SshTunnel, Some("operator"), None, Some(PEM), None)
                .is_ok()
        );
        assert!(
            validate_tunnel_auth(
                Provider::SshTunnel,
                Some("operator"),
                Some("s3cret"),
                Some(PEM),
                Some("phrase")
            )
            .is_ok()
        );
        // A passphrase without a key protects nothing.
        assert!(
            validate_tunnel_auth(
                Provider::SshTunnel,
                Some("operator"),
                Some("s3cret"),
                None,
                Some("phrase")
            )
            .is_err()
        );
        // Structural PEM check only; real parsing is the tunnel engine's job.
        assert!(
            validate_tunnel_auth(
                Provider::SshTunnel,
                Some("operator"),
                None,
                Some("not a key"),
                None
            )
            .is_err()
        );
        assert!(validate_private_key_pem(PEM).is_ok());
        assert!(validate_private_key_pem("-----BEGIN X-----\nbody").is_err());
    }

    #[test]
    fn only_tunnels_take_key_material() {
        const PEM: &str =
            "-----BEGIN OPENSSH PRIVATE KEY-----\nbody\n-----END OPENSSH PRIVATE KEY-----";
        for provider in [
            Provider::Http,
            Provider::Socks4,
            Provider::Socks5,
            Provider::Trawl,
        ] {
            assert!(
                validate_tunnel_auth(provider, Some("operator"), None, Some(PEM), None).is_err(),
                "{provider:?} must reject a private key"
            );
            assert!(
                validate_tunnel_auth(provider, Some("operator"), None, None, Some("phrase"))
                    .is_err(),
                "{provider:?} must reject a passphrase"
            );
            assert!(validate_tunnel_auth(provider, Some("operator"), None, None, None).is_ok());
        }
    }

    #[test]
    fn tunnels_speak_no_solver_protocol_and_choose_no_dns() {
        assert_eq!(
            resolve_new_proxy_protocol(Provider::SshTunnel, None).expect("no protocol"),
            None
        );
        assert!(
            resolve_new_proxy_protocol(
                Provider::SshTunnel,
                Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1)
            )
            .is_err()
        );
        assert!(resolve_remote_dns(Provider::SshTunnel, Some(true), None).is_err());
        assert!(!resolve_remote_dns(Provider::SshTunnel, Some(false), None).expect("off is fine"));
    }

    #[test]
    fn protocol_is_required_for_solvers_and_forbidden_for_transports() {
        assert_eq!(
            resolve_new_proxy_protocol(Provider::Trawl, None).expect("solver default"),
            Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1)
        );
        assert_eq!(
            resolve_new_proxy_protocol(Provider::Socks5, None).expect("transports carry none"),
            None
        );
        assert!(
            resolve_new_proxy_protocol(
                Provider::Http,
                Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1),
            )
            .is_err()
        );
    }

    #[test]
    fn credentials_are_transport_only_and_need_a_username() {
        assert!(validate_proxy_credentials(Provider::Socks5, Some("user"), Some("pass")).is_ok());
        assert!(validate_proxy_credentials(Provider::Trawl, None, None).is_ok());
        assert!(validate_proxy_credentials(Provider::Trawl, Some("user"), None).is_err());
        assert!(validate_proxy_credentials(Provider::Http, None, Some("pass")).is_err());
        assert_eq!(normalize_proxy_credential(Some("  ".into())), None);
        assert_eq!(
            normalize_proxy_credential(Some(" user ".into())),
            Some("user".to_string())
        );
    }

    fn transport_config(provider_type: Provider, base_url: &str) -> scryer_domain::ProxyConfig {
        let now = Utc::now();
        scryer_domain::ProxyConfig {
            id: "transport-1".into(),
            name: "Transport".into(),
            provider_type,
            protocol: None,
            base_url: base_url.to_string(),
            request_timeout_seconds: 5,
            is_enabled: true,
            username_encrypted: None,
            password_encrypted: None,
            remote_dns: false,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            private_key_encrypted: None,
            private_key_passphrase_encrypted: None,
        }
    }

    #[test]
    fn remote_dns_selects_the_socks5h_scheme_for_the_client() {
        let mut config = transport_config(Provider::Socks5, "socks5://gateway:1080");
        assert_eq!(
            effective_transport_proxy_url(&config),
            "socks5://gateway:1080"
        );
        config.remote_dns = true;
        assert_eq!(
            effective_transport_proxy_url(&config),
            "socks5h://gateway:1080"
        );
    }

    #[test]
    fn socks_endpoints_default_to_port_1080() {
        let config = transport_config(Provider::Socks5, "socks5://gateway");
        assert_eq!(
            transport_proxy_endpoint(&config).expect("socks default port"),
            ("gateway".to_string(), 1080)
        );
        let config = transport_config(Provider::Http, "http://gateway");
        assert_eq!(
            transport_proxy_endpoint(&config).expect("http default port"),
            ("gateway".to_string(), 80)
        );
    }

    #[tokio::test]
    async fn transport_probe_reports_an_unreachable_proxy_endpoint() {
        // Bind and immediately drop, so the port is almost certainly closed.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener should bind");
        let address = listener.local_addr().expect("listener address");
        drop(listener);

        let config = transport_config(Provider::Socks5, &format!("socks5://{address}"));
        let error = probe_transport_proxy_health(&config, None)
            .await
            .expect_err("a closed port is not a working proxy");

        assert!(
            error
                .to_string()
                .contains(TRANSPORT_PROXY_UNREACHABLE_MESSAGE),
            "unexpected message: {error}"
        );
    }

    #[tokio::test]
    async fn transport_probe_without_an_assigned_indexer_checks_the_endpoint_only() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener should bind");
        let address = listener.local_addr().expect("listener address");
        let accepted = tokio::spawn(async move { listener.accept().await.map(|_| ()).is_ok() });

        let config = transport_config(Provider::Socks5, &format!("socks5://{address}"));
        let message = probe_transport_proxy_health(&config, None)
            .await
            .expect("a listening proxy endpoint passes the reachability check");

        assert!(accepted.await.expect("listener task"));
        assert!(
            message.contains("assign an indexer"),
            "the probe must say what it did not verify: {message}"
        );
    }

    #[tokio::test]
    async fn transport_probe_reports_downstream_failures_separately() {
        // A listener that accepts and hangs up is reachable but cannot carry a
        // request, which is exactly the split the messages have to express.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener should bind");
        let address = listener.local_addr().expect("listener address");
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                drop(stream);
            }
        });

        let config = transport_config(Provider::Http, &format!("http://{address}"));
        let error = probe_transport_proxy_health(&config, Some("http://indexer.test/api"))
            .await
            .expect_err("a proxy that hangs up cannot carry the request");

        let message = error.to_string();
        assert!(
            message.contains(TRANSPORT_PROXY_DOWNSTREAM_MESSAGE),
            "reachable-but-broken must not read as unreachable: {message}"
        );
        assert!(!message.contains(TRANSPORT_PROXY_UNREACHABLE_MESSAGE));
    }

    #[tokio::test]
    async fn transport_probe_carries_a_request_through_an_http_proxy() {
        // wiremock answers absolute-form requests the same way it answers
        // origin-form ones, which is enough to prove the client dialled the
        // proxy rather than the destination.
        let proxy = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(407))
            .mount(&proxy)
            .await;

        let config = transport_config(Provider::Http, &proxy.uri());
        let message = probe_transport_proxy_health(&config, Some("http://indexer.test/api"))
            .await
            .expect("any HTTP answer proves the tunnel carries traffic");

        assert!(
            message.contains("HTTP 407"),
            "the probe reports the status rather than judging it: {message}"
        );
        let requests = proxy.received_requests().await.expect("recorded requests");
        assert_eq!(requests.len(), 1);
    }

    fn test_config(
        server: &MockServer,
        provider_type: scryer_domain::ProxyProviderType,
    ) -> scryer_domain::ProxyConfig {
        let now = Utc::now();
        scryer_domain::ProxyConfig {
            id: "proxy-1".into(),
            name: "Solver".into(),
            provider_type,
            protocol: Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1),
            username_encrypted: None,
            password_encrypted: None,
            remote_dns: false,
            base_url: server.uri(),
            request_timeout_seconds: 60,
            is_enabled: true,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            private_key_encrypted: None,
            private_key_passphrase_encrypted: None,
        }
    }

    fn assert_browser_user_agent(request: &wiremock::Request) {
        assert_eq!(
            request
                .headers
                .get("user-agent")
                .and_then(|value| value.to_str().ok()),
            Some(scryer_outbound_http::PROXY_USER_AGENT)
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
            scryer_domain::ProxyProviderType::Trawl,
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
            scryer_domain::ProxyProviderType::Trawl,
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
        let config = scryer_domain::ProxyConfig {
            id: "trawl-transport-failure".into(),
            name: "Trawl".into(),
            provider_type: scryer_domain::ProxyProviderType::Trawl,
            protocol: Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1),
            username_encrypted: None,
            password_encrypted: None,
            remote_dns: false,
            base_url: format!("http://{address}"),
            request_timeout_seconds: 60,
            is_enabled: true,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            private_key_encrypted: None,
            private_key_passphrase_encrypted: None,
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
            scryer_domain::ProxyProviderType::Trawl,
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
            scryer_domain::ProxyProviderType::Trawl,
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
            scryer_domain::ProxyProviderType::Trawl,
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
            scryer_domain::ProxyProviderType::Trawl,
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
            scryer_domain::ProxyProviderType::Trawl,
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
