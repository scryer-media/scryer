use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use scryer_application::challenge_solver as solver;
use scryer_application::transport_proxy;
use scryer_application::{
    AppError, AppResult, CONNECTION_TEST_INDEXER_ID, CapturedIndexerHttpResponse,
    IndexerConfigRepository, IndexerErrorOperation, IndexerPluginProvider, IndexerRequestResult,
    IndexerRequestTally, IndexerStatsTracker, NullIndexerStatsTracker, ProxyConfigRepository,
    RateLimitCooldownAction, ResolvedDownloadArtifact, extract_magnet_info_hash,
    is_valid_magnet_uri, normalize_torrent_info_hash,
};
use scryer_domain::{IndexerConfig, ProxyConfig};
use scryer_outbound_http::{
    DestinationKey, OutboundHttpClient, OutboundHttpError, PluginEgressPolicy, RateLimitRegistry,
    RequestPolicy, generic_reqwest_client, prepare_plugin_http_target,
    prepare_plugin_http_target_from_url, proxy_reqwest_client,
};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

const ARTIFACT_MAX_BYTES: usize = 32 * 1024 * 1024;
const SOLVER_RESPONSE_MAX_BYTES: usize = ARTIFACT_MAX_BYTES * 2;

/// A successful artifact HTTP response whose body has not been buffered.
pub struct ArtifactHttpResponse {
    pub response: reqwest::Response,
    pub tally: Box<IndexerRequestTally>,
    pub final_url: Option<String>,
    pub headers: Option<serde_json::Value>,
}

pub enum ArtifactFetchResponse {
    Http(ArtifactHttpResponse),
    Resolved(ResolvedDownloadArtifact),
}

/// Indexer-owned artifact transport. It intentionally owns solver selection and
/// artifact HTTP so download-client routing never acquires indexer credentials.
#[derive(Clone)]
pub struct IndexerArtifactTransport {
    outbound_http: OutboundHttpClient,
    proxy_clients: Arc<Mutex<HashMap<String, ArtifactProxyClient>>>,
    indexer_configs: Arc<dyn IndexerConfigRepository>,
    proxy_configs: Arc<dyn ProxyConfigRepository>,
    indexer_stats: Arc<dyn IndexerStatsTracker>,
    /// Lets an indexer that owns an authenticated grab flow resolve its own
    /// artifacts ahead of the host's direct, transport-proxy, and
    /// challenge-solver fetches.
    indexer_plugin_provider: Option<Arc<dyn IndexerPluginProvider>>,
}

const MAX_ARTIFACT_PROXY_CLIENTS: usize = 32;
const ARTIFACT_PROXY_CLIENT_TTL: Duration = Duration::from_secs(300);

struct ArtifactProxyClient {
    revision: String,
    endpoint: String,
    created_at: Instant,
    client: reqwest::Client,
}

/// How one artifact hop leaves the host.
#[derive(Clone, Copy)]
enum ArtifactEgress<'a> {
    /// Resolve and pin every hop through the guarded plugin target.
    Direct,
    /// Dial the operator's transport proxy instead. The destination is
    /// validated syntactically but not resolved here, because with
    /// `remote_dns` that name belongs to the proxy and may not resolve locally
    /// at all. The caller has already confirmed the URL matches the assigned
    /// indexer's origin.
    TransportProxy(&'a ProxyConfig),
}

/// How an indexer's assigned proxy takes part in its artifact fetches.
enum ArtifactProxyRoute {
    Direct,
    /// A transport or tunnel proxy solves nothing: the fetch is the direct
    /// fetch, dialled through the operator's proxy instead of straight out.
    TransportProxy(ProxyConfig),
    ChallengeSolver(ProxyConfig),
}

impl ArtifactProxyRoute {
    fn proxy(&self) -> Option<&ProxyConfig> {
        match self {
            Self::Direct => None,
            Self::TransportProxy(proxy) | Self::ChallengeSolver(proxy) => Some(proxy),
        }
    }
}

impl IndexerArtifactTransport {
    pub fn new(
        indexer_configs: Arc<dyn IndexerConfigRepository>,
        proxy_configs: Arc<dyn ProxyConfigRepository>,
    ) -> Self {
        Self {
            outbound_http: OutboundHttpClient::new(
                generic_reqwest_client(),
                RateLimitRegistry::new(),
            ),
            proxy_clients: Arc::new(Mutex::new(HashMap::new())),
            indexer_configs,
            proxy_configs,
            indexer_stats: Arc::new(NullIndexerStatsTracker),
            indexer_plugin_provider: None,
        }
    }

    pub fn with_indexer_stats_tracker(mut self, stats: Arc<dyn IndexerStatsTracker>) -> Self {
        self.indexer_stats = stats;
        self
    }

    /// Enable indexer-owned grab resolution ahead of the host's direct,
    /// transport-proxy, and challenge-solver fetches. Indexers with no grab
    /// flow answer `None` and leave those paths unchanged.
    pub fn with_indexer_plugin_provider(
        mut self,
        indexer_plugin_provider: Arc<dyn IndexerPluginProvider>,
    ) -> Self {
        self.indexer_plugin_provider = Some(indexer_plugin_provider);
        self
    }

    /// Drain proxy health observations recorded by the fetches above into the
    /// repository. One drain, shared with the search path.
    pub async fn flush_proxy_health(&self) {
        solver::flush_solver_health(self.proxy_configs.as_ref()).await;
    }

    /// Resolve how the indexer's assigned proxy, if any, takes part in an
    /// artifact fetch. Fail-closed: an assignment that cannot be loaded or is
    /// disabled is an error, never "carry on unproxied".
    async fn proxy_route(&self, config: &IndexerConfig) -> AppResult<ArtifactProxyRoute> {
        let Some(proxy_id) = config.proxy_config_id.as_deref() else {
            return Ok(ArtifactProxyRoute::Direct);
        };
        let proxy = self
            .proxy_configs
            .get_by_id(proxy_id)
            .await?
            .ok_or_else(|| {
                AppError::DownloadSubmitUnavailable(
                    "The assigned indexer proxy is no longer available.".into(),
                )
            })?;
        // Prowlarr owns challenge handling for its parent and synchronized
        // children, so a challenge solver assigned there is ignored. Transport
        // and tunnel proxies only carry bytes and still apply.
        if proxy.is_challenge_solver()
            && (config.is_prowlarr_nab_proxy()
                || config.provider_type.trim().eq_ignore_ascii_case("prowlarr"))
        {
            return Ok(ArtifactProxyRoute::Direct);
        }
        if !proxy.is_enabled {
            return Err(AppError::DownloadSubmitUnavailable(
                "The assigned indexer proxy is disabled.".into(),
            ));
        }
        Ok(if proxy.is_challenge_solver() {
            ArtifactProxyRoute::ChallengeSolver(proxy)
        } else {
            ArtifactProxyRoute::TransportProxy(proxy)
        })
    }

    /// An indexer that owns an authenticated grab flow resolves the artifact
    /// itself: a private tracker will not serve the file to a bare fetch, and
    /// the plugin already holds that session. Providers without such a flow
    /// answer `None` and leave the transport paths untouched.
    async fn indexer_owned_artifact(
        &self,
        config: &IndexerConfig,
        proxy: Option<&ProxyConfig>,
        download_url: &str,
    ) -> AppResult<Option<ResolvedDownloadArtifact>> {
        let Some(provider) = self.indexer_plugin_provider.as_ref() else {
            return Ok(None);
        };
        let Some(client) = provider.client_for_provider_with_proxy(config, proxy) else {
            return Ok(None);
        };
        client.resolve_download(download_url).await
    }

    /// Reuse connection pools across artifact requests and redirect hops. The
    /// cache belongs to this transport, carries no indexer credentials, and
    /// never bypasses the route lookup or the redirect loop's per-hop checks.
    fn proxy_client(&self, proxy: &ProxyConfig) -> Result<reqwest::Client, String> {
        let revision = transport_proxy::transport_proxy_revision(proxy);
        // A restarted tunnel front must invalidate the pool even if settings
        // did not change. SSH also rechecks the current host-key decision here.
        let endpoint = transport_proxy::proxy_egress_url(proxy)?;
        let mut clients = self
            .proxy_clients
            .lock()
            .expect("artifact proxy client cache lock");
        clients.retain(|_, entry| entry.created_at.elapsed() < ARTIFACT_PROXY_CLIENT_TTL);
        if let Some(entry) = clients.get(&proxy.id)
            && entry.revision == revision
            && entry.endpoint == endpoint
        {
            return Ok(entry.client.clone());
        }
        clients.remove(&proxy.id);
        let client = transport_proxy::transport_proxied_reqwest_client_with_redirect_policy(
            proxy,
            "",
            reqwest::redirect::Policy::none(),
        )?;
        if clients.len() >= MAX_ARTIFACT_PROXY_CLIENTS
            && let Some(oldest) = clients
                .iter()
                .min_by_key(|(_, entry)| entry.created_at)
                .map(|(id, _)| id.clone())
        {
            clients.remove(&oldest);
        }
        clients.insert(
            proxy.id.clone(),
            ArtifactProxyClient {
                revision,
                endpoint,
                created_at: Instant::now(),
                client: client.clone(),
            },
        );
        Ok(client)
    }

    /// Pick the client and URL for one redirect hop under the chosen egress.
    async fn hop_target(
        &self,
        current: &url::Url,
        remaining: Duration,
        policy: &PluginEgressPolicy,
        egress: ArtifactEgress<'_>,
    ) -> AppResult<(reqwest::Client, url::Url)> {
        match egress {
            ArtifactEgress::Direct => {
                let target = timeout(
                    remaining,
                    prepare_plugin_http_target_from_url(
                        current.clone(),
                        "indexer download artifact",
                        policy,
                    ),
                )
                .await
                .map_err(|_| {
                    AppError::DownloadSubmitUnavailable(
                        "The download artifact fetch timed out.".into(),
                    )
                })?
                .map_err(|_| {
                    AppError::DownloadSubmitUnavailable(
                        "Scryer refused an unsafe download artifact destination.".into(),
                    )
                })?;
                Ok((target.client().clone(), target.url().clone()))
            }
            ArtifactEgress::TransportProxy(proxy) => {
                let url = scryer_outbound_http::validate_operator_http_url(
                    current.as_str(),
                    "indexer download artifact",
                )
                .map_err(|error| {
                    tracing::warn!(error = %error, "blocked unsafe indexer download artifact URL");
                    AppError::DownloadSubmitUnavailable(
                        "Scryer refused an unsafe download artifact destination.".into(),
                    )
                })?;
                let client = self.proxy_client(proxy).map_err(|message| {
                    transport_proxy::record_transport_proxy_failure(proxy, &message);
                    AppError::DownloadSubmitUnavailable(message)
                })?;
                Ok((client, url))
            }
        }
    }

    fn tally(&self, config: Option<&IndexerConfig>) -> IndexerRequestTally {
        IndexerRequestTally::new(
            config
                .map(|config| config.id.clone())
                .unwrap_or_else(|| CONNECTION_TEST_INDEXER_ID.to_string()),
            config
                .map(|config| config.name.clone())
                .unwrap_or_else(|| "indexer".to_string()),
            IndexerErrorOperation::IndexerAction,
            Arc::clone(&self.indexer_stats),
        )
    }

    /// Fetch a directly accessible artifact without buffering its body. Solver
    /// routes retain their existing buffered path because a solver may return
    /// the artifact inline rather than through a response stream.
    pub async fn fetch_response(
        &self,
        indexer_id: Option<&str>,
        download_url: &str,
    ) -> AppResult<Option<ArtifactFetchResponse>> {
        self.fetch_response_with_cancellation(indexer_id, download_url, &CancellationToken::new())
            .await
    }

    pub async fn fetch_response_with_cancellation(
        &self,
        indexer_id: Option<&str>,
        download_url: &str,
        cancellation: &CancellationToken,
    ) -> AppResult<Option<ArtifactFetchResponse>> {
        tokio::select! {
            _ = cancellation.cancelled() => Err(artifact_fetch_cancelled()),
            result = self.fetch_response_inner(indexer_id, download_url) => result,
        }
    }

    async fn fetch_response_inner(
        &self,
        indexer_id: Option<&str>,
        download_url: &str,
    ) -> AppResult<Option<ArtifactFetchResponse>> {
        let config = match indexer_id.map(str::trim).filter(|id| !id.is_empty()) {
            Some(id) => self.indexer_configs.get_by_id(id).await?.ok_or_else(|| {
                AppError::DownloadSubmitUnavailable(
                    "The indexer for this download artifact is no longer available.".into(),
                )
            })?,
            None => {
                let tally = self.tally(None);
                return self
                    .fetch_response_direct(
                        "indexer",
                        download_url,
                        scryer_outbound_http::STANDARD_HTTP_TIMEOUT,
                        &PluginEgressPolicy::default(),
                        None,
                        None,
                        tally,
                        ArtifactEgress::Direct,
                    )
                    .await
                    .map(Some);
            }
        };
        let policy = PluginEgressPolicy::for_operator_configured_url(&config.base_url);
        let destination_cooldown_key = DestinationKey::from(config.rate_limit_domain_key());
        // Resolve the assigned proxy first: both the origin guard and the
        // indexer-owned grab below need it, and a misconfigured proxy must fail
        // the same way whichever path ends up resolving the artifact.
        let route = self.proxy_route(&config).await?;
        if route.proxy().is_some() && !same_origin(&config, download_url) {
            return Err(AppError::Validation(
                "Proxied download URL does not match the assigned indexer origin.".into(),
            ));
        }
        if let Some(artifact) = self
            .indexer_owned_artifact(&config, route.proxy(), download_url)
            .await?
        {
            return Ok(Some(ArtifactFetchResponse::Resolved(artifact)));
        }
        let tally = self.tally(Some(&config));
        match route {
            ArtifactProxyRoute::Direct => self
                .fetch_response_direct(
                    &config.name,
                    download_url,
                    scryer_outbound_http::STANDARD_HTTP_TIMEOUT,
                    &policy,
                    Some(&config.base_url),
                    Some(destination_cooldown_key),
                    tally,
                    ArtifactEgress::Direct,
                )
                .await
                .map(Some),
            ArtifactProxyRoute::TransportProxy(proxy) => {
                let response = self
                    .fetch_response_direct(
                        solver::solver_provider_name(proxy.provider_type),
                        download_url,
                        scryer_outbound_http::effective_proxy_request_timeout(
                            proxy.request_timeout_seconds,
                        ),
                        &policy,
                        Some(&config.base_url),
                        Some(destination_cooldown_key),
                        tally,
                        ArtifactEgress::TransportProxy(&proxy),
                    )
                    .await?;
                transport_proxy::record_transport_proxy_success(&proxy);
                Ok(Some(response))
            }
            // Solver routes keep the buffered path: a solver may return the
            // artifact inline rather than through a response stream.
            ArtifactProxyRoute::ChallengeSolver(_) => Ok(None),
        }
    }

    pub async fn resolve(
        &self,
        indexer_id: Option<&str>,
        download_url: &str,
        info_hash_hint: Option<String>,
    ) -> AppResult<ResolvedDownloadArtifact> {
        self.resolve_with_cancellation(
            indexer_id,
            download_url,
            info_hash_hint,
            &CancellationToken::new(),
        )
        .await
    }

    pub async fn resolve_with_cancellation(
        &self,
        indexer_id: Option<&str>,
        download_url: &str,
        info_hash_hint: Option<String>,
        cancellation: &CancellationToken,
    ) -> AppResult<ResolvedDownloadArtifact> {
        tokio::select! {
            _ = cancellation.cancelled() => Err(artifact_fetch_cancelled()),
            result = self.resolve_inner(indexer_id, download_url, info_hash_hint) => result,
        }
    }

    async fn resolve_inner(
        &self,
        indexer_id: Option<&str>,
        download_url: &str,
        info_hash_hint: Option<String>,
    ) -> AppResult<ResolvedDownloadArtifact> {
        let config = match indexer_id.map(str::trim).filter(|id| !id.is_empty()) {
            Some(id) => self.indexer_configs.get_by_id(id).await?.ok_or_else(|| {
                AppError::DownloadSubmitUnavailable(
                    "The indexer for this download artifact is no longer available.".into(),
                )
            })?,
            None => {
                let tally = self.tally(None);
                return self
                    .fetch_and_classify(
                        "indexer",
                        download_url,
                        &[],
                        scryer_outbound_http::STANDARD_HTTP_TIMEOUT,
                        &PluginEgressPolicy::default(),
                        None,
                        None,
                        &tally,
                        info_hash_hint,
                        ArtifactEgress::Direct,
                    )
                    .await;
            }
        };
        let policy = PluginEgressPolicy::for_operator_configured_url(&config.base_url);
        let destination_cooldown_key = DestinationKey::from(config.rate_limit_domain_key());
        let tally = self.tally(Some(&config));
        let route = self.proxy_route(&config).await?;
        if route.proxy().is_some() && !same_origin(&config, download_url) {
            return Err(AppError::Validation(
                "Proxied download URL does not match the assigned indexer origin.".into(),
            ));
        }
        match route {
            ArtifactProxyRoute::Direct => {
                self.fetch_and_classify(
                    &config.name,
                    download_url,
                    &[],
                    scryer_outbound_http::STANDARD_HTTP_TIMEOUT,
                    &policy,
                    Some(&config.base_url),
                    Some(destination_cooldown_key.clone()),
                    &tally,
                    info_hash_hint,
                    ArtifactEgress::Direct,
                )
                .await
            }
            ArtifactProxyRoute::TransportProxy(proxy) => {
                let artifact = self
                    .fetch_and_classify(
                        solver::solver_provider_name(proxy.provider_type),
                        download_url,
                        &[],
                        scryer_outbound_http::effective_proxy_request_timeout(
                            proxy.request_timeout_seconds,
                        ),
                        &policy,
                        Some(&config.base_url),
                        Some(destination_cooldown_key.clone()),
                        &tally,
                        info_hash_hint,
                        ArtifactEgress::TransportProxy(&proxy),
                    )
                    .await?;
                transport_proxy::record_transport_proxy_success(&proxy);
                Ok(artifact)
            }
            ArtifactProxyRoute::ChallengeSolver(proxy) => {
                self.resolve_via_solver(
                    &proxy,
                    download_url,
                    info_hash_hint,
                    &policy,
                    &config.base_url,
                    destination_cooldown_key,
                    &tally,
                )
                .await
            }
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "preserve independent egress, deadline, and accounting policy through solver resolution"
    )]
    async fn resolve_via_solver(
        &self,
        proxy: &scryer_domain::ProxyConfig,
        url: &str,
        hint: Option<String>,
        policy: &PluginEgressPolicy,
        indexer_base_url: &str,
        destination_cooldown_key: DestinationKey,
        tally: &IndexerRequestTally,
    ) -> AppResult<ResolvedDownloadArtifact> {
        let provider = proxy.provider_type;
        let name = solver::solver_provider_name(provider);
        prepare_plugin_http_target(url, "indexer download artifact", policy)
            .await
            .map_err(|_| {
                AppError::DownloadSubmitUnavailable(
                    "Scryer refused an unsafe download artifact destination.".into(),
                )
            })?;
        let headers = solver::SolvedSessionCache::shared().session_headers(&proxy.id, url);
        let timeout =
            scryer_outbound_http::effective_proxy_request_timeout(proxy.request_timeout_seconds);
        match self
            .fetch_and_classify(
                name,
                url,
                &headers,
                timeout,
                policy,
                Some(indexer_base_url),
                Some(destination_cooldown_key.clone()),
                tally,
                hint.clone(),
                ArtifactEgress::Direct,
            )
            .await
        {
            Ok(artifact) => return Ok(artifact),
            Err(error @ AppError::TemporaryUnavailable { .. }) => return Err(error),
            Err(error @ AppError::NewznabQuotaExceeded { .. })
            | Err(error @ AppError::DownloadSourceGone(_)) => return Err(error),
            Err(error) if matches!(&error, AppError::Validation(message) if message.contains("Newznab error 910")) =>
            {
                return Err(error);
            }
            Err(_) => {}
        }
        if !headers.is_empty() {
            solver::SolvedSessionCache::shared().invalidate(&proxy.id, url);
        }
        let solver_http = OutboundHttpClient::new(proxy_reqwest_client(), RateLimitRegistry::new());
        let response = tokio::time::timeout(
            timeout,
            solver_http.send_with_rate_limit_and_dispatch_observer(
                RequestPolicy::no_retry("indexer_solver", "indexer_artifact_solver")
                    .without_redirects(),
                || {
                    solver_http
                        .client()
                        .post(solver::solver_solve_endpoint(&proxy.base_url))
                        .timeout(timeout)
                        .json(&solver::solver_solve_request(
                            provider,
                            url,
                            proxy.request_timeout_seconds,
                        ))
                },
                |_| async {},
                |_| {
                    // The solver replays this request against the indexer, so
                    // it is one indexer request regardless of the POST's origin.
                    tally
                        .try_note_sent_call(scryer_application::CALL_SOLVER)
                        .then_some(())
                        .ok_or(OutboundHttpError::DispatchRejected)
                },
            ),
        )
        .await
        .map_err(|_| {
            AppError::DownloadSubmitUnavailable(
                solver::solver_error_message(provider, solver::SolverErrorKind::Unreachable).into(),
            )
        })?
        .map_err(|error| {
            if matches!(error, OutboundHttpError::Transport { .. }) {
                tally.note_result(IndexerRequestResult::NoResponse);
            }
            AppError::DownloadSubmitUnavailable(
                solver::solver_error_message(provider, solver::SolverErrorKind::Unreachable).into(),
            )
        })?;
        let status = response.status();
        let body = read_body(response, SOLVER_RESPONSE_MAX_BYTES)
            .await
            .map_err(|_| {
                AppError::DownloadSubmitUnavailable(
                    solver::solver_error_message(provider, solver::SolverErrorKind::Unreadable)
                        .into(),
                )
            })?;
        let solution = solver::parse_solver_solution(&body)
            .map_err(|e| AppError::DownloadSubmitUnavailable(e.message(provider).into()))?;
        solver::SolvedSessionCache::shared().store_solution(&proxy.id, url, &solution);
        let solution_status = solution.status.unwrap_or(status.as_u16());
        if solution_status == 429
            || solver::solved_body_looks_rate_limited(
                solution.response.as_deref().unwrap_or_default().as_bytes(),
            )
        {
            return Err(rate_limited(solution.headers.as_ref()));
        }
        let retry_headers = solver::solution_retry_headers(&solution);
        if !(200..300).contains(&solution_status)
            || should_refetch(url, solution.url.as_deref(), solution.headers.as_ref())
        {
            if retry_headers.is_empty() {
                return Err(AppError::DownloadSubmitUnavailable(format!(
                    "{name} target request returned HTTP {solution_status}."
                )));
            }
            return self
                .fetch_and_classify(
                    name,
                    url,
                    &retry_headers,
                    timeout,
                    policy,
                    Some(indexer_base_url),
                    Some(destination_cooldown_key),
                    tally,
                    hint,
                    ArtifactEgress::Direct,
                )
                .await;
        }
        classify(
            name,
            solution.url.as_deref(),
            solution.headers.as_ref(),
            solution.response.unwrap_or_default().into_bytes(),
            hint,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "carry artifact hints alongside the explicit transport policy"
    )]
    async fn fetch_and_classify(
        &self,
        name: &str,
        url: &str,
        headers: &[(String, String)],
        request_timeout: Duration,
        policy: &PluginEgressPolicy,
        indexer_base_url: Option<&str>,
        destination_cooldown_key: Option<DestinationKey>,
        tally: &IndexerRequestTally,
        hint: Option<String>,
        egress: ArtifactEgress<'_>,
    ) -> AppResult<ResolvedDownloadArtifact> {
        let fetched = self
            .fetch(
                name,
                url,
                headers,
                request_timeout,
                policy,
                indexer_base_url,
                destination_cooldown_key,
                tally,
                egress,
            )
            .await?;
        classify(
            name,
            fetched.2.as_deref(),
            fetched.1.as_ref(),
            fetched.0,
            hint,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "redirect hops retain the explicit egress, deadline, session, and accounting policy"
    )]
    async fn fetch(
        &self,
        name: &str,
        raw: &str,
        session_headers: &[(String, String)],
        request_timeout: Duration,
        policy: &PluginEgressPolicy,
        indexer_base_url: Option<&str>,
        destination_cooldown_key: Option<DestinationKey>,
        tally: &IndexerRequestTally,
        egress: ArtifactEgress<'_>,
    ) -> AppResult<(Vec<u8>, Option<serde_json::Value>, Option<String>)> {
        let original = url::Url::parse(raw)
            .map_err(|_| AppError::Validation("Download artifact URL is invalid.".into()))?;
        let origin = (
            original.scheme().to_string(),
            original.host_str().map(str::to_string),
            original.port_or_known_default(),
        );
        let deadline = Instant::now() + request_timeout;
        let mut current = original;
        let mut visited = HashSet::from([current.clone()]);
        for hops in 0..=5 {
            if current.scheme() == "magnet" && is_valid_magnet_uri(current.as_str()) {
                return Ok((Vec::new(), None, Some(current.to_string())));
            }
            if !matches!(current.scheme(), "http" | "https") {
                return Err(AppError::Validation(
                    "Download artifact redirects must use HTTP(S) or magnet URLs.".into(),
                ));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(AppError::DownloadSubmitUnavailable(
                    "The download artifact fetch timed out.".into(),
                ));
            }
            let (hop_client, hop_url) =
                self.hop_target(&current, remaining, policy, egress).await?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(AppError::DownloadSubmitUnavailable(
                    "The download artifact fetch timed out.".into(),
                ));
            }
            let same = origin.0 == hop_url.scheme()
                && origin
                    .1
                    .as_deref()
                    .zip(hop_url.host_str())
                    .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
                && origin.2 == hop_url.port_or_known_default();
            let mut builder = hop_client.get(hop_url.clone()).timeout(remaining);
            if same {
                for (k, v) in session_headers {
                    builder = builder.header(k, v);
                }
            }
            let request_policy =
                RequestPolicy::no_retry("indexer_artifact", "indexer_artifact_fetch")
                    .without_redirects();
            let request_policy = match destination_cooldown_key.clone() {
                Some(destination) => request_policy.with_destination_cooldown_key(destination),
                None => request_policy,
            };
            let response = timeout(
                remaining,
                self.outbound_http
                    .send_with_rate_limit_and_dispatch_observer(
                        request_policy,
                        || {
                            builder
                                .try_clone()
                                .expect("artifact GET requests are cloneable")
                        },
                        |_| async {},
                        |actual_url| {
                            let indexer_request = indexer_base_url
                                .and_then(|base| same_url_origin(base, actual_url))
                                .unwrap_or(true);
                            let admitted = if indexer_request {
                                tally.try_note_sent(actual_url.as_str())
                            } else {
                                tally.try_note_auxiliary_sent_call("external_redirect")
                            };
                            admitted
                                .then_some(())
                                .ok_or(OutboundHttpError::DispatchRejected)
                        },
                    ),
            )
            .await
            .map_err(|_| {
                AppError::DownloadSubmitUnavailable("The download artifact fetch timed out.".into())
            })?
            .map_err(|error| map_artifact_outbound_error_via(name, tally, error, egress))?;
            if response.status().is_redirection() {
                if hops == 5 {
                    return Err(AppError::Validation(
                        "The download artifact exceeded the redirect limit.".into(),
                    ));
                }
                let loc = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| {
                        AppError::Validation(
                            "Download artifact redirect was missing a location.".into(),
                        )
                    })?;
                let next = current
                    .join(loc)
                    .or_else(|_| url::Url::parse(loc))
                    .map_err(|_| {
                        AppError::Validation(
                            "Download artifact redirect location was invalid.".into(),
                        )
                    })?;
                if !visited.insert(next.clone()) {
                    return Err(AppError::Validation(
                        "The download artifact redirect looped.".into(),
                    ));
                }
                current = next;
                continue;
            }
            if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let retry_after = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| scryer_outbound_http::parse_retry_after(v).map(|(d, _)| d));
                return Err(AppError::TemporaryUnavailable {
                    message: solver::rate_limit_message_with_retry_after(retry_after),
                    retry_after,
                    rate_limit_cooldown: RateLimitCooldownAction::AlreadyRecorded,
                });
            }
            if matches!(
                response.status(),
                reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE
            ) {
                return Err(AppError::DownloadSourceGone(format!(
                    "The download artifact fetch returned HTTP {}.",
                    response.status()
                )));
            }
            if !response.status().is_success() {
                return Err(AppError::DownloadSubmitUnavailable(format!(
                    "The download artifact fetch returned HTTP {}.",
                    response.status()
                )));
            }
            let final_url = Some(response.url().to_string());
            let headers = selected_headers(&response);
            let bytes = read_body(response, ARTIFACT_MAX_BYTES).await.map_err(|_| {
                AppError::DownloadSubmitUnavailable(
                    "Scryer could not read the download artifact.".into(),
                )
            })?;
            return Ok((bytes, headers, final_url));
        }
        Err(AppError::Validation(
            "The download artifact redirect loop was exhausted.".into(),
        ))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "streaming handoff retains the explicit egress, deadline, and accounting policy"
    )]
    async fn fetch_response_direct(
        &self,
        name: &str,
        raw: &str,
        request_timeout: Duration,
        policy: &PluginEgressPolicy,
        indexer_base_url: Option<&str>,
        destination_cooldown_key: Option<DestinationKey>,
        tally: IndexerRequestTally,
        egress: ArtifactEgress<'_>,
    ) -> AppResult<ArtifactFetchResponse> {
        let mut current = url::Url::parse(raw)
            .map_err(|_| AppError::Validation("Download artifact URL is invalid.".into()))?;
        let deadline = Instant::now() + request_timeout;
        let mut visited = HashSet::from([current.clone()]);
        for hops in 0..=5 {
            if !matches!(current.scheme(), "http" | "https") {
                return Err(AppError::Validation(
                    "Download artifact redirects must use HTTP(S) URLs.".into(),
                ));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(AppError::DownloadSubmitUnavailable(
                    "The download artifact fetch timed out.".into(),
                ));
            }
            let (hop_client, hop_url) =
                self.hop_target(&current, remaining, policy, egress).await?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(AppError::DownloadSubmitUnavailable(
                    "The download artifact fetch timed out.".into(),
                ));
            }
            let request_policy =
                RequestPolicy::no_retry("indexer_artifact", "indexer_artifact_fetch")
                    .without_redirects();
            let request_policy = match destination_cooldown_key.clone() {
                Some(destination) => request_policy.with_destination_cooldown_key(destination),
                None => request_policy,
            };
            let response = timeout(
                remaining,
                self.outbound_http
                    .send_with_rate_limit_and_dispatch_observer(
                        request_policy,
                        || hop_client.get(hop_url.clone()).timeout(remaining),
                        |_| async {},
                        |actual_url| {
                            let indexer_request = indexer_base_url
                                .and_then(|base| same_url_origin(base, actual_url))
                                .unwrap_or(true);
                            let admitted = if indexer_request {
                                tally.try_note_sent(actual_url.as_str())
                            } else {
                                tally.try_note_auxiliary_sent_call("external_redirect")
                            };
                            admitted
                                .then_some(())
                                .ok_or(OutboundHttpError::DispatchRejected)
                        },
                    ),
            )
            .await
            .map_err(|_| {
                tally.note_result(IndexerRequestResult::NoResponse);
                AppError::DownloadSubmitUnavailable("The download artifact fetch timed out.".into())
            })?
            .map_err(|error| map_artifact_outbound_error_via(name, &tally, error, egress))?;
            if !response.status().is_success() {
                tally.note_response(&CapturedIndexerHttpResponse {
                    status: response.status().as_u16(),
                    headers: Vec::new(),
                    body: Vec::new(),
                });
            }
            if response.status().is_redirection() {
                if hops == 5 {
                    return Err(AppError::Validation(
                        "The download artifact exceeded the redirect limit.".into(),
                    ));
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| {
                        AppError::Validation(
                            "Download artifact redirect was missing a location.".into(),
                        )
                    })?;
                let next = current
                    .join(location)
                    .or_else(|_| url::Url::parse(location))
                    .map_err(|_| {
                        AppError::Validation(
                            "Download artifact redirect location was invalid.".into(),
                        )
                    })?;
                if next.scheme() == "magnet" && is_valid_magnet_uri(next.as_str()) {
                    let uri = next.to_string();
                    return Ok(ArtifactFetchResponse::Resolved(
                        ResolvedDownloadArtifact::Magnet {
                            info_hash_hint: extract_magnet_info_hash(&uri),
                            uri,
                        },
                    ));
                }
                if !visited.insert(next.clone()) {
                    return Err(AppError::Validation(
                        "The download artifact redirect looped.".into(),
                    ));
                }
                current = next;
                continue;
            }
            if matches!(
                response.status(),
                reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE
            ) {
                return Err(AppError::DownloadSourceGone(format!(
                    "The download artifact fetch returned HTTP {}.",
                    response.status()
                )));
            }
            if !response.status().is_success() {
                return Err(AppError::DownloadSubmitUnavailable(format!(
                    "The download artifact fetch returned HTTP {}.",
                    response.status()
                )));
            }
            let final_url = Some(response.url().to_string());
            let headers = selected_headers(&response);
            return Ok(ArtifactFetchResponse::Http(ArtifactHttpResponse {
                response,
                tally: Box::new(tally),
                final_url,
                headers,
            }));
        }
        Err(AppError::Validation(
            "The download artifact redirect loop was exhausted.".into(),
        ))
    }
}

/// `map_artifact_outbound_error`, with connector failures on a proxied hop
/// attributed to the proxy rather than the indexer, by name.
fn map_artifact_outbound_error_via(
    name: &str,
    tally: &IndexerRequestTally,
    error: OutboundHttpError,
    egress: ArtifactEgress<'_>,
) -> AppError {
    if let ArtifactEgress::TransportProxy(proxy) = egress
        && let OutboundHttpError::Transport { source, .. } = &error
        && !source.is_timeout()
        && let Some(message) = transport_proxy::transport_proxy_connect_failure(proxy, source)
    {
        tally.note_result(IndexerRequestResult::NoResponse);
        transport_proxy::record_transport_proxy_failure(proxy, &message);
        return AppError::DownloadSubmitUnavailable(message);
    }
    map_artifact_outbound_error(name, tally, error)
}

fn map_artifact_outbound_error(
    name: &str,
    tally: &IndexerRequestTally,
    error: OutboundHttpError,
) -> AppError {
    match error {
        OutboundHttpError::RateLimited(rate_limited) => AppError::TemporaryUnavailable {
            message: "The download artifact destination is rate limited.".into(),
            retry_after: rate_limited.retry_after,
            rate_limit_cooldown: RateLimitCooldownAction::AlreadyRecorded,
        },
        OutboundHttpError::Transport { source, .. } if source.is_timeout() => {
            tally.note_result(IndexerRequestResult::NoResponse);
            AppError::DownloadSubmitUnavailable("The download artifact fetch timed out.".into())
        }
        OutboundHttpError::Transport { .. } => {
            tally.note_result(IndexerRequestResult::NoResponse);
            AppError::DownloadSubmitUnavailable(format!(
                "Scryer could not fetch the download artifact from '{name}'."
            ))
        }
        OutboundHttpError::DispatchRejected => AppError::DownloadSubmitUnavailable(format!(
            "Scryer could not fetch the download artifact from '{name}'."
        )),
    }
}

fn artifact_fetch_cancelled() -> AppError {
    AppError::TemporaryUnavailable {
        message: "nzb artifact resolution was cancelled".into(),
        retry_after: None,
        rate_limit_cooldown: RateLimitCooldownAction::None,
    }
}

fn selected_headers(response: &reqwest::Response) -> Option<serde_json::Value> {
    let mut map = serde_json::Map::new();
    for name in ["content-type", "content-disposition"] {
        if let Some(value) = response.headers().get(name).and_then(|v| v.to_str().ok()) {
            map.insert(name.to_string(), value.into());
        }
    }
    (!map.is_empty()).then_some(serde_json::Value::Object(map))
}
async fn read_body(mut response: reqwest::Response, max: usize) -> Result<Vec<u8>, ()> {
    if response.content_length().is_some_and(|n| n > max as u64) {
        return Err(());
    }
    let mut out = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if out.len().checked_add(chunk.len()).ok_or(())? > max {
            return Err(());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}
fn same_origin(config: &IndexerConfig, raw: &str) -> bool {
    let Ok(url) = url::Url::parse(raw) else {
        return false;
    };
    let Ok(base) = url::Url::parse(&config.base_url) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
        && url.scheme() == base.scheme()
        && url.port_or_known_default() == base.port_or_known_default()
        && url
            .host_str()
            .zip(base.host_str())
            .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn same_url_origin(base: &str, actual: &reqwest::Url) -> Option<bool> {
    let base = url::Url::parse(base).ok()?;
    Some(
        base.scheme() == actual.scheme()
            && base.port_or_known_default() == actual.port_or_known_default()
            && base
                .host_str()
                .zip(actual.host_str())
                .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b)),
    )
}
fn rate_limited(headers: Option<&serde_json::Value>) -> AppError {
    let retry_after = solver::retry_after_from_solution_headers(headers);
    AppError::TemporaryUnavailable {
        message: solver::rate_limit_message_with_retry_after(retry_after),
        retry_after,
        rate_limit_cooldown: RateLimitCooldownAction::RecordFallback,
    }
}
fn should_refetch(
    original: &str,
    final_url: Option<&str>,
    headers: Option<&serde_json::Value>,
) -> bool {
    solver::solution_header_string(headers, "content-type")
        .is_some_and(|v| v.contains("bittorrent") || v.contains("octet-stream"))
        || [Some(original), final_url]
            .into_iter()
            .flatten()
            .any(|raw| {
                url::Url::parse(raw)
                    .ok()
                    .is_some_and(|u| u.path().to_ascii_lowercase().ends_with(".torrent"))
            })
}
pub(crate) fn classify(
    name: &str,
    final_url: Option<&str>,
    headers: Option<&serde_json::Value>,
    bytes: Vec<u8>,
    hint: Option<String>,
) -> AppResult<ResolvedDownloadArtifact> {
    if final_url.is_some_and(is_valid_magnet_uri)
        || std::str::from_utf8(&bytes)
            .ok()
            .is_some_and(|v| is_valid_magnet_uri(v.trim()))
    {
        let uri = final_url
            .filter(|url| is_valid_magnet_uri(url))
            .unwrap_or_else(|| std::str::from_utf8(&bytes).unwrap().trim())
            .to_string();
        return Ok(ResolvedDownloadArtifact::Magnet {
            info_hash_hint: extract_magnet_info_hash(&uri).or(hint),
            uri,
        });
    }
    let captured = CapturedIndexerHttpResponse {
        status: 200,
        headers: Vec::new(),
        body: bytes[..bytes.len().min(64 * 1024)].to_vec(),
    };
    if let Some(error) = scryer_application::classify_indexer_http_response(&captured) {
        match error.provider_error_code {
            Some(code @ (500 | 501)) => {
                return Err(AppError::NewznabQuotaExceeded {
                    code,
                    message: error.message.to_string(),
                });
            }
            Some(910) => {
                return Err(AppError::Validation(format!(
                    "{name} returned Newznab error 910: {}.",
                    error.message
                )));
            }
            _ => {}
        }
    }
    let content_type = solver::solution_header_string(headers, "content-type");
    let file_name = solver::solution_header_string(headers, "content-disposition").and_then(|v| {
        v.split(';').find_map(|p| {
            p.trim()
                .strip_prefix("filename=")
                .map(|x| x.trim_matches('"').to_string())
        })
    });
    if looks_nzb(&bytes)
        || content_type
            .as_deref()
            .is_some_and(|v| v.to_ascii_lowercase().starts_with("application/x-nzb"))
    {
        if !looks_nzb(&bytes) {
            return Err(AppError::Validation(format!(
                "{name} resolved invalid NZB bytes."
            )));
        }
        return Ok(ResolvedDownloadArtifact::Nzb {
            bytes,
            file_name,
            content_type,
        });
    }
    let torrent = torrent_hash(&bytes);
    if torrent.is_some() {
        let info_hash_hint = if torrent.as_ref().is_some_and(|hash| hash.len() == 40)
            && hint
                .as_deref()
                .and_then(|h| normalize_torrent_info_hash(Some(h)))
                .is_some_and(|h| h.len() == 64)
        {
            hint
        } else {
            torrent.or(hint)
        };
        return Ok(ResolvedDownloadArtifact::TorrentFile {
            bytes,
            file_name,
            content_type,
            info_hash_hint,
        });
    }
    Err(AppError::Validation(format!(
        "{name} resolved the download URL, but the result was not an NZB, magnet URI, or torrent file."
    )))
}
fn looks_nzb(bytes: &[u8]) -> bool {
    std::str::from_utf8(&bytes[..bytes.len().min(4096)])
        .ok()
        .is_some_and(|s| {
            let s = s.trim_start().to_ascii_lowercase();
            (s.starts_with("<?xml") && s.contains("<nzb"))
                || s.starts_with("<nzb")
                || s.contains("<!doctype nzb")
        })
}
fn torrent_hash(bytes: &[u8]) -> Option<String> {
    if bytes.len() > ARTIFACT_MAX_BYTES || !bytes.starts_with(b"d") {
        return None;
    }
    let (consumed, info) = parse_bencode_dict(bytes, 0, 0).ok()?;
    if consumed != bytes.len() {
        return None;
    }
    let (start, end) = info?;
    let mut cursor = start + 1;
    let mut meta_version = None;
    let mut has_v1_pieces = false;
    while cursor < end - 1 {
        let (value_start, key_start, key_end) = parse_bencode_string(bytes, cursor).ok()?;
        let value_end = parse_bencode_value(bytes, value_start, 1).ok()?;
        match &bytes[key_start..key_end] {
            b"meta version" => {
                if meta_version
                    .replace(&bytes[value_start..value_end])
                    .is_some()
                {
                    return None;
                }
            }
            b"pieces" => has_v1_pieces = true,
            _ => {}
        }
        cursor = value_end;
    }
    if meta_version.is_some_and(|version| version != b"i2e") {
        return None;
    }
    // Hybrids retain their v1 identity; pure v2 metadata has no v1 piece list.
    let algorithm = if meta_version.is_some() && !has_v1_pieces {
        &aws_lc_rs::digest::SHA256
    } else {
        &aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY
    };
    let digest = aws_lc_rs::digest::digest(algorithm, &bytes[start..end]);
    Some(digest.as_ref().iter().map(|b| format!("{b:02x}")).collect())
}

fn parse_bencode_value(bytes: &[u8], offset: usize, depth: usize) -> Result<usize, ()> {
    if depth > 64 || offset >= bytes.len() {
        return Err(());
    }
    match bytes[offset] {
        b'i' => {
            let end = bytes[offset + 1..]
                .iter()
                .position(|b| *b == b'e')
                .map(|p| offset + p + 1)
                .ok_or(())?;
            let integer = &bytes[offset + 1..end];
            let digits = integer.strip_prefix(b"-").unwrap_or(integer);
            if digits.is_empty()
                || !digits.iter().all(u8::is_ascii_digit)
                || (digits.len() > 1 && digits[0] == b'0')
                || integer == b"-0"
            {
                return Err(());
            }
            Ok(end + 1)
        }
        b'l' => {
            let mut cursor = offset + 1;
            while bytes.get(cursor) != Some(&b'e') {
                cursor = parse_bencode_value(bytes, cursor, depth + 1)?;
            }
            Ok(cursor + 1)
        }
        b'd' => parse_bencode_dict(bytes, offset, depth + 1).map(|(cursor, _)| cursor),
        b'0'..=b'9' => parse_bencode_string(bytes, offset).map(|(cursor, _, _)| cursor),
        _ => Err(()),
    }
}

fn parse_bencode_dict(
    bytes: &[u8],
    offset: usize,
    depth: usize,
) -> Result<(usize, Option<(usize, usize)>), ()> {
    if depth > 64 || bytes.get(offset) != Some(&b'd') {
        return Err(());
    }
    let mut cursor = offset + 1;
    let mut info = None;
    while bytes.get(cursor) != Some(&b'e') {
        let (after_key, start, end) = parse_bencode_string(bytes, cursor)?;
        let is_info = depth == 0 && &bytes[start..end] == b"info";
        if is_info && bytes.get(after_key) != Some(&b'd') {
            return Err(());
        }
        let after_value = parse_bencode_value(bytes, after_key, depth + 1)?;
        if is_info && info.replace((after_key, after_value)).is_some() {
            return Err(());
        }
        cursor = after_value;
    }
    Ok((cursor + 1, info))
}

fn parse_bencode_string(bytes: &[u8], offset: usize) -> Result<(usize, usize, usize), ()> {
    let colon = bytes[offset..]
        .iter()
        .position(|b| *b == b':')
        .map(|p| offset + p)
        .ok_or(())?;
    if colon == offset {
        return Err(());
    }
    let length = std::str::from_utf8(&bytes[offset..colon])
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .ok_or(())?;
    let start = colon + 1;
    let end = start.checked_add(length).ok_or(())?;
    if end > bytes.len() {
        return Err(());
    }
    Ok((end, start, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::Utc;
    use scryer_application::{IndexerConfigUpdate, IndexerQueryStats};
    use std::sync::atomic::{AtomicU32, Ordering};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    struct ChallengeThenArtifact(AtomicU32);

    impl Respond for ChallengeThenArtifact {
        fn respond(&self, _: &Request) -> ResponseTemplate {
            if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(500)
            } else {
                ResponseTemplate::new(200).set_body_bytes(b"<nzb></nzb>")
            }
        }
    }

    #[derive(Default)]
    struct TestConfigRepository {
        configs: Vec<IndexerConfig>,
    }

    #[async_trait]
    impl IndexerConfigRepository for TestConfigRepository {
        async fn list(&self, _provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>> {
            Ok(self.configs.clone())
        }
        async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>> {
            Ok(self.configs.iter().find(|config| config.id == id).cloned())
        }
        async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
            Ok(config)
        }
        async fn touch_last_error(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }
        async fn update(&self, _update: IndexerConfigUpdate) -> AppResult<IndexerConfig> {
            Err(AppError::Validation(
                "not used by artifact transport tests".into(),
            ))
        }
        async fn delete(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct CountingStats {
        sent: AtomicU32,
        gate: Option<scryer_application::IndexerDispatchGate>,
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn artifact_proxy_pool_reuses_connections_and_expires_on_profile_edits() {
        use scryer_tunnel::test_support::{CLIENT_ED25519_PEM, SshServerDouble, SshServerOptions};
        let ssh = SshServerDouble::start(SshServerOptions::default()).await;
        let origin = MockServer::start().await;
        Mock::given(path("/artifact"))
            .respond_with(ResponseTemplate::new(200).set_body_string("artifact"))
            .mount(&origin)
            .await;
        Mock::given(path("/redirect"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/artifact"))
            .mount(&origin)
            .await;
        let transport = transport(Arc::new(CountingStats::default()));
        let mut proxy = test_proxy(&format!("ssh://{}", ssh.addr()), true);
        proxy.id = "artifact-proxy-pool".into();
        proxy.provider_type = scryer_domain::ProxyProviderType::SshTunnel;
        proxy.protocol = None;
        proxy.username_encrypted = Some("operator".into());
        proxy.private_key_encrypted = Some(CLIENT_ED25519_PEM.into());
        for _ in 0..3 {
            let client = transport.proxy_client(&proxy).unwrap();
            assert_eq!(
                client
                    .get(format!("{}/artifact", origin.uri()))
                    .send()
                    .await
                    .unwrap()
                    .text()
                    .await
                    .unwrap(),
                "artifact"
            );
        }
        assert_eq!(
            ssh.forwarded_targets().len(),
            1,
            "three requests share a single SSH channel and TCP connection"
        );
        let client = transport.proxy_client(&proxy).unwrap();
        let response = client
            .get(format!("{}/redirect", origin.uri()))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            302,
            "redirect policy remains owned by the artifact loop"
        );
        response.bytes().await.unwrap();
        proxy.updated_at += chrono::Duration::seconds(1);
        let client = transport.proxy_client(&proxy).unwrap();
        assert_eq!(
            client
                .get(format!("{}/artifact", origin.uri()))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "artifact"
        );
        assert_eq!(ssh.forwarded_targets().len(), 2);
        {
            let mut clients = transport.proxy_clients.lock().unwrap();
            clients.get_mut(&proxy.id).unwrap().created_at -= ARTIFACT_PROXY_CLIENT_TTL;
        }
        let client = transport.proxy_client(&proxy).unwrap();
        assert_eq!(
            client
                .get(format!("{}/artifact", origin.uri()))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "artifact"
        );
        assert_eq!(ssh.forwarded_targets().len(), 3);
        scryer_application::tunnel_proxy::stop_tunnel(&proxy.id);
        for i in 0..40 {
            let mut proxy = test_proxy("http://127.0.0.1:1", true);
            proxy.id = format!("bounded-cache-{i}");
            proxy.provider_type = scryer_domain::ProxyProviderType::Http;
            proxy.protocol = None;
            transport.proxy_client(&proxy).unwrap();
        }
        assert_eq!(
            transport.proxy_clients.lock().unwrap().len(),
            MAX_ARTIFACT_PROXY_CLIENTS
        );
    }

    impl IndexerStatsTracker for CountingStats {
        fn dispatch_gate(&self) -> Option<scryer_application::IndexerDispatchGate> {
            self.gate.clone()
        }
        fn record_query(&self, _: &str, _: &str, _: bool) {}
        fn record_grab(&self, _: &str, _: &str) {}
        fn record_api_limits(
            &self,
            _: &str,
            _: Option<u32>,
            _: Option<u32>,
            _: Option<u32>,
            _: Option<u32>,
        ) {
        }
        fn record_api_request_sent(&self, _: &str, _: &str) {
            self.sent.fetch_add(1, Ordering::SeqCst);
        }
        fn all_stats(&self) -> Vec<IndexerQueryStats> {
            Vec::new()
        }
    }

    #[derive(Default)]
    struct ResultRecorder(std::sync::Mutex<Vec<(String, Arc<std::sync::atomic::AtomicU64>)>>);

    struct RecordedCounter(Arc<std::sync::atomic::AtomicU64>);
    impl metrics::CounterFn for RecordedCounter {
        fn increment(&self, value: u64) {
            self.0
                .fetch_add(value, std::sync::atomic::Ordering::Relaxed);
        }
        fn absolute(&self, value: u64) {
            self.0
                .fetch_max(value, std::sync::atomic::Ordering::Relaxed);
        }
    }
    impl metrics::Recorder for ResultRecorder {
        fn describe_counter(
            &self,
            _: metrics::KeyName,
            _: Option<metrics::Unit>,
            _: metrics::SharedString,
        ) {
        }
        fn describe_gauge(
            &self,
            _: metrics::KeyName,
            _: Option<metrics::Unit>,
            _: metrics::SharedString,
        ) {
        }
        fn describe_histogram(
            &self,
            _: metrics::KeyName,
            _: Option<metrics::Unit>,
            _: metrics::SharedString,
        ) {
        }
        fn register_counter(
            &self,
            key: &metrics::Key,
            _: &metrics::Metadata<'_>,
        ) -> metrics::Counter {
            if key.name() != scryer_application::INDEXER_API_REQUESTS_METRIC {
                return metrics::Counter::noop();
            }
            let result = key
                .labels()
                .find(|label| label.key() == "result")
                .unwrap()
                .value()
                .to_string();
            let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
            self.0.lock().unwrap().push((result, counter.clone()));
            metrics::Counter::from_arc(Arc::new(RecordedCounter(counter)))
        }
        fn register_gauge(&self, _: &metrics::Key, _: &metrics::Metadata<'_>) -> metrics::Gauge {
            metrics::Gauge::noop()
        }
        fn register_histogram(
            &self,
            _: &metrics::Key,
            _: &metrics::Metadata<'_>,
        ) -> metrics::Histogram {
            metrics::Histogram::noop()
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn streaming_artifact_timeout_records_no_response() {
        let recorder = ResultRecorder::default();
        let _recording = metrics::set_default_local_recorder(&recorder);
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(1)))
            .mount(&server)
            .await;
        let stats = Arc::new(CountingStats {
            sent: AtomicU32::new(0),
            gate: None,
        });
        let transport = transport(stats.clone());
        let tally = IndexerRequestTally::new(
            "id".into(),
            "fixture".into(),
            IndexerErrorOperation::IndexerAction,
            stats,
        );
        let result = transport
            .fetch_response_direct(
                "fixture",
                &server.uri(),
                Duration::from_millis(100),
                &PluginEgressPolicy::default(),
                None,
                None,
                tally,
                ArtifactEgress::Direct,
            )
            .await;
        assert!(matches!(
            result,
            Err(AppError::DownloadSubmitUnavailable(_))
        ));
        let recorded = recorder.0.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "no_response");
        assert_eq!(recorded[0].1.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn streaming_artifact_tally_survives_handoff_and_classifies_body() {
        for (body, outcome, succeeds) in [
            ("<nzb/>", "success", true),
            ("d4:infod4:name6:Titleaee", "success", true),
            (
                "<error code=\"500\" description=\"limit\"/>",
                "newznab_request_limit_reached",
                false,
            ),
        ] {
            let recorder = ResultRecorder::default();
            let _recording = metrics::set_default_local_recorder(&recorder);
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_string(body))
                .expect(1)
                .mount(&server)
                .await;
            let stats = Arc::new(CountingStats {
                sent: AtomicU32::new(0),
                gate: None,
            });
            let response = transport(stats)
                .fetch_response(None, &server.uri())
                .await
                .unwrap()
                .unwrap();
            let super::ArtifactFetchResponse::Http(response) = response else {
                panic!("HTTP response");
            };
            assert!(
                recorder.0.lock().unwrap().is_empty(),
                "tally must stay pending through handoff"
            );
            let temp = tempfile::tempdir().unwrap();
            let store: Arc<dyn scryer_application::StagedNzbStore> = Arc::new(
                crate::downloads::staged_nzb_store::FileSystemStagedNzbStore::new(temp.path())
                    .await
                    .unwrap(),
            );
            let result = crate::indexers::artifact_staging::stage_or_buffer_nzb_response(
                response.response,
                Some(&response.tally),
                &store,
                &Arc::new(tokio::sync::Semaphore::new(1)),
                "fixture",
                None,
                None,
                &tokio_util::sync::CancellationToken::new(),
            )
            .await;
            assert_eq!(result.is_ok(), succeeds);
            drop(response.tally);
            let recorded: Vec<_> = recorder
                .0
                .lock()
                .unwrap()
                .iter()
                .map(|(label, count)| {
                    (
                        label.clone(),
                        count.load(std::sync::atomic::Ordering::Relaxed),
                    )
                })
                .collect();
            assert_eq!(recorded, vec![(outcome.to_string(), 1)]);
        }
    }

    fn transport(stats: Arc<CountingStats>) -> IndexerArtifactTransport {
        IndexerArtifactTransport::new(
            Arc::new(TestConfigRepository::default()),
            Arc::new(scryer_application::NullProxyConfigRepository),
        )
        .with_indexer_stats_tracker(stats)
    }

    fn test_config(base_url: &str, proxy_id: Option<&str>) -> IndexerConfig {
        let now = Utc::now();
        IndexerConfig {
            id: "indexer".into(),
            name: "fixture".into(),
            provider_type: "newznab".into(),
            base_url: base_url.into(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            proxy_config_id: proxy_id.map(str::to_string),
            download_client_id: None,
            seeding_profile_id: None,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn transport_with_config(
        stats: Arc<CountingStats>,
        config: IndexerConfig,
    ) -> IndexerArtifactTransport {
        IndexerArtifactTransport::new(
            Arc::new(TestConfigRepository {
                configs: vec![config],
            }),
            Arc::new(scryer_application::NullProxyConfigRepository),
        )
        .with_indexer_stats_tracker(stats)
    }

    fn transport_with_config_and_proxy(
        stats: Arc<CountingStats>,
        config: IndexerConfig,
        proxy: scryer_domain::ProxyConfig,
    ) -> IndexerArtifactTransport {
        IndexerArtifactTransport::new(
            Arc::new(TestConfigRepository {
                configs: vec![config],
            }),
            Arc::new(SingleProxyRepository(proxy)),
        )
        .with_indexer_stats_tracker(stats)
    }

    fn test_proxy(base_url: &str, is_enabled: bool) -> scryer_domain::ProxyConfig {
        let now = Utc::now();
        scryer_domain::ProxyConfig {
            id: "solver".into(),
            name: "fixture solver".into(),
            provider_type: scryer_domain::ProxyProviderType::Byparr,
            protocol: Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1),
            base_url: base_url.into(),
            request_timeout_seconds: 5,
            is_enabled,
            username_encrypted: None,
            password_encrypted: None,
            remote_dns: false,
            private_key_encrypted: None,
            private_key_passphrase_encrypted: None,
            peer_public_key: None,
            preshared_key_encrypted: None,
            tunnel_public_key: None,
            tunnel_addresses: Vec::new(),
            tunnel_dns_servers: Vec::new(),
            tunnel_mtu: None,
            tunnel_keepalive_seconds: None,
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    struct SingleProxyRepository(scryer_domain::ProxyConfig);

    #[async_trait]
    impl ProxyConfigRepository for SingleProxyRepository {
        async fn list(
            &self,
            _: Option<scryer_domain::ProxyProviderType>,
        ) -> AppResult<Vec<scryer_domain::ProxyConfig>> {
            Ok(vec![self.0.clone()])
        }
        async fn get_by_id(&self, id: &str) -> AppResult<Option<scryer_domain::ProxyConfig>> {
            Ok((id == self.0.id).then(|| self.0.clone()))
        }
        async fn create(
            &self,
            config: scryer_domain::ProxyConfig,
        ) -> AppResult<scryer_domain::ProxyConfig> {
            Ok(config)
        }
        async fn update(
            &self,
            config: scryer_domain::ProxyConfig,
        ) -> AppResult<scryer_domain::ProxyConfig> {
            Ok(config)
        }
        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
        async fn record_health(
            &self,
            _: &str,
            _: scryer_domain::ProxyHealthStatus,
            _: Option<String>,
            _: Option<chrono::DateTime<Utc>>,
        ) -> AppResult<()> {
            Ok(())
        }
        async fn pin_host_key(
            &self,
            _: &str,
            _: &str,
            _: chrono::DateTime<Utc>,
            _: chrono::DateTime<Utc>,
        ) -> AppResult<bool> {
            Ok(true)
        }
        async fn clear_host_key(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn missing_assigned_solver_fails_before_artifact_http() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let stats = Arc::new(CountingStats {
            sent: AtomicU32::new(0),
            gate: None,
        });
        let transport = transport_with_config(
            Arc::clone(&stats),
            test_config(&server.uri(), Some("missing")),
        );
        let result = transport
            .resolve(Some("indexer"), &format!("{}/grab", server.uri()), None)
            .await;
        let Err(error) = result else {
            panic!("missing solver must fail");
        };
        assert!(error.to_string().contains("proxy is no longer available"));
        assert!(server.received_requests().await.unwrap().is_empty());
        assert_eq!(stats.sent.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn metadata_destination_is_rejected_before_dispatch() {
        let stats = Arc::new(CountingStats {
            sent: AtomicU32::new(0),
            gate: None,
        });
        let transport = transport_with_config(
            Arc::clone(&stats),
            test_config("http://169.254.169.254", None),
        );
        let result = transport
            .resolve(
                Some("indexer"),
                "http://169.254.169.254/latest/meta-data/",
                None,
            )
            .await;
        let Err(error) = result else {
            panic!("metadata destination must fail");
        };
        assert!(
            error
                .to_string()
                .contains("unsafe download artifact destination")
        );
        assert_eq!(stats.sent.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn disabled_assigned_solver_fails_before_artifact_http() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let stats = Arc::new(CountingStats {
            sent: AtomicU32::new(0),
            gate: None,
        });
        let transport = transport_with_config_and_proxy(
            Arc::clone(&stats),
            test_config(&server.uri(), Some("solver")),
            test_proxy(&server.uri(), false),
        );
        let result = transport
            .resolve(Some("indexer"), &format!("{}/grab", server.uri()), None)
            .await;
        let Err(error) = result else {
            panic!("disabled solver must fail");
        };
        assert!(error.to_string().contains("proxy is disabled"));
        assert!(server.received_requests().await.unwrap().is_empty());
        assert_eq!(stats.sent.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn assigned_solver_rejects_full_origin_mismatch_before_artifact_http() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let stats = Arc::new(CountingStats {
            sent: AtomicU32::new(0),
            gate: None,
        });
        let transport = transport_with_config_and_proxy(
            Arc::clone(&stats),
            test_config(&server.uri(), Some("solver")),
            test_proxy(&server.uri(), true),
        );
        let result = transport
            .resolve(Some("indexer"), "http://127.0.0.1:1/grab", None)
            .await;
        let Err(error) = result else {
            panic!("full origin mismatch must fail");
        };
        assert!(
            error
                .to_string()
                .contains("does not match the assigned indexer origin")
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        assert_eq!(stats.sent.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn configured_hop_and_external_redirect_have_exact_dashboard_count() {
        let external = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/final"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"<nzb></nzb>"))
            .mount(&external)
            .await;
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/grab"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/final", external.uri())),
            )
            .mount(&server)
            .await;
        let stats = Arc::new(CountingStats {
            sent: AtomicU32::new(0),
            gate: None,
        });
        let transport = transport(Arc::clone(&stats));
        let tally = IndexerRequestTally::new(
            "id".into(),
            "fixture".into(),
            IndexerErrorOperation::IndexerAction,
            stats.clone(),
        );
        let response = transport
            .fetch_response_direct(
                "fixture",
                &format!("{}/grab", server.uri()),
                Duration::from_secs(5),
                &PluginEgressPolicy::for_operator_configured_url(&server.uri()),
                Some(&server.uri()),
                None,
                tally,
                ArtifactEgress::Direct,
            )
            .await
            .expect("redirect fetch");
        drop(response);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert_eq!(external.received_requests().await.unwrap().len(), 1);
        assert_eq!(stats.sent.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn closed_gate_prevents_artifact_http() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let gate = scryer_application::IndexerDispatchGate::default();
        gate.close();
        let stats = Arc::new(CountingStats {
            gate: Some(gate),
            ..Default::default()
        });
        let transport = transport(Arc::clone(&stats));
        let tally = IndexerRequestTally::new(
            "id".into(),
            "fixture".into(),
            IndexerErrorOperation::IndexerAction,
            stats.clone(),
        );
        let result = transport
            .fetch_response_direct(
                "fixture",
                &server.uri(),
                Duration::from_secs(5),
                &PluginEgressPolicy::default(),
                None,
                None,
                tally,
                ArtifactEgress::Direct,
            )
            .await;
        let Err(error) = result else {
            panic!("closed gate must reject before HTTP");
        };
        assert!(matches!(error, AppError::DownloadSubmitUnavailable(_)));
        assert!(server.received_requests().await.unwrap().is_empty());
        assert_eq!(stats.sent.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn redirects_resolve_magnets_and_reject_loops() {
        let server = MockServer::start().await;
        let magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567";
        Mock::given(method("GET"))
            .and(path("/magnet"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", magnet))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/a"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", "/b"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/b"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", "/a"))
            .mount(&server)
            .await;
        let stats = Arc::new(CountingStats {
            sent: AtomicU32::new(0),
            gate: None,
        });
        let transport = transport(Arc::clone(&stats));
        let tally = IndexerRequestTally::new(
            "id".into(),
            "fixture".into(),
            IndexerErrorOperation::IndexerAction,
            stats.clone(),
        );
        let resolved = transport
            .fetch_response_direct(
                "fixture",
                &format!("{}/magnet", server.uri()),
                Duration::from_secs(5),
                &PluginEgressPolicy::default(),
                None,
                None,
                tally,
                ArtifactEgress::Direct,
            )
            .await
            .expect("magnet redirect");
        assert!(matches!(
            resolved,
            ArtifactFetchResponse::Resolved(ResolvedDownloadArtifact::Magnet { .. })
        ));
        let tally = IndexerRequestTally::new(
            "id".into(),
            "fixture".into(),
            IndexerErrorOperation::IndexerAction,
            stats,
        );
        let loop_result = transport
            .fetch_response_direct(
                "fixture",
                &format!("{}/a", server.uri()),
                Duration::from_secs(5),
                &PluginEgressPolicy::default(),
                None,
                None,
                tally,
                ArtifactEgress::Direct,
            )
            .await;
        let Err(error) = loop_result else {
            panic!("loop must fail");
        };
        assert!(error.to_string().contains("redirect looped"));
    }

    #[tokio::test]
    async fn cross_origin_redirect_strips_session_headers() {
        let target = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/final"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"<nzb></nzb>"))
            .mount(&target)
            .await;
        let origin = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/grab"))
            .and(header("cookie", "secret=yes"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/final", target.uri())),
            )
            .mount(&origin)
            .await;
        let stats = Arc::new(CountingStats {
            sent: AtomicU32::new(0),
            gate: None,
        });
        let transport = transport(Arc::clone(&stats));
        let tally = IndexerRequestTally::new(
            "id".into(),
            "fixture".into(),
            IndexerErrorOperation::IndexerAction,
            stats,
        );
        transport
            .fetch(
                "fixture",
                &format!("{}/grab", origin.uri()),
                &[("Cookie".into(), "secret=yes".into())],
                Duration::from_secs(5),
                &PluginEgressPolicy::default(),
                Some(&origin.uri()),
                None,
                &tally,
                ArtifactEgress::Direct,
            )
            .await
            .expect("fetch succeeds");
        let target_request = target.received_requests().await.unwrap().pop().unwrap();
        assert!(target_request.headers.get("cookie").is_none());
    }

    #[tokio::test]
    async fn quota_response_with_assigned_solver_does_not_dispatch_solver() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/grab"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"<error code="500"/>"#))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let stats = Arc::new(CountingStats {
            sent: AtomicU32::new(0),
            gate: None,
        });
        let transport = transport(Arc::clone(&stats));
        let now = Utc::now();
        let proxy = scryer_domain::ProxyConfig {
            id: "solver".into(),
            name: "fixture solver".into(),
            provider_type: scryer_domain::ProxyProviderType::Byparr,
            protocol: Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1),
            base_url: server.uri(),
            request_timeout_seconds: 5,
            is_enabled: true,
            username_encrypted: None,
            password_encrypted: None,
            remote_dns: false,
            private_key_encrypted: None,
            private_key_passphrase_encrypted: None,
            peer_public_key: None,
            preshared_key_encrypted: None,
            tunnel_public_key: None,
            tunnel_addresses: Vec::new(),
            tunnel_dns_servers: Vec::new(),
            tunnel_mtu: None,
            tunnel_keepalive_seconds: None,
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
        };
        let tally = IndexerRequestTally::new(
            "id".into(),
            "fixture".into(),
            IndexerErrorOperation::IndexerAction,
            stats.clone(),
        );
        let policy = PluginEgressPolicy::for_operator_configured_url(&server.uri());
        let quota_result = transport
            .resolve_via_solver(
                &proxy,
                &format!("{}/grab", server.uri()),
                None,
                &policy,
                &server.uri(),
                DestinationKey::from("fixture"),
                &tally,
            )
            .await;
        let Err(error) = quota_result else {
            panic!("quota must propagate");
        };
        assert!(matches!(
            error,
            AppError::NewznabQuotaExceeded { code: 500, .. }
        ));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert_eq!(stats.sent.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn assigned_solver_preserves_rate_limit_source_gone_and_api_disabled_without_solver() {
        for (status, body, label) in [
            (429, "", "rate limit"),
            (404, "", "not found"),
            (410, "", "gone"),
            (200, r#"<error code="501"/>"#, "grab quota"),
            (200, r#"<error code="910"/>"#, "api disabled"),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/grab"))
                .respond_with(ResponseTemplate::new(status).set_body_string(body))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/v1"))
                .respond_with(ResponseTemplate::new(500))
                .expect(0)
                .mount(&server)
                .await;
            let stats = Arc::new(CountingStats {
                sent: AtomicU32::new(0),
                gate: None,
            });
            let transport = transport(Arc::clone(&stats));
            let now = Utc::now();
            let proxy = scryer_domain::ProxyConfig {
                id: "solver".into(),
                name: "fixture solver".into(),
                provider_type: scryer_domain::ProxyProviderType::Byparr,
                protocol: Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1),
                base_url: server.uri(),
                request_timeout_seconds: 5,
                is_enabled: true,
                username_encrypted: None,
                password_encrypted: None,
                remote_dns: false,
                private_key_encrypted: None,
                private_key_passphrase_encrypted: None,
                peer_public_key: None,
                preshared_key_encrypted: None,
                tunnel_public_key: None,
                tunnel_addresses: Vec::new(),
                tunnel_dns_servers: Vec::new(),
                tunnel_mtu: None,
                tunnel_keepalive_seconds: None,
                host_key_fingerprint: None,
                host_key_pinned_at: None,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                created_at: now,
                updated_at: now,
            };
            let tally = IndexerRequestTally::new(
                "id".into(),
                "fixture".into(),
                IndexerErrorOperation::IndexerAction,
                stats.clone(),
            );
            let policy = PluginEgressPolicy::for_operator_configured_url(&server.uri());
            let result = transport
                .resolve_via_solver(
                    &proxy,
                    &format!("{}/grab", server.uri()),
                    None,
                    &policy,
                    &server.uri(),
                    DestinationKey::from("fixture"),
                    &tally,
                )
                .await;
            let Err(error) = result else {
                panic!("{label} must propagate");
            };
            match status {
                429 => assert!(matches!(error, AppError::TemporaryUnavailable { .. })),
                404 | 410 => assert!(matches!(error, AppError::DownloadSourceGone(_))),
                _ if body.contains("501") => assert!(matches!(
                    error,
                    AppError::NewznabQuotaExceeded { code: 501, .. }
                )),
                _ => assert!(
                    matches!(error, AppError::Validation(message) if message.contains("910"))
                ),
            }
            assert_eq!(
                server.received_requests().await.unwrap().len(),
                1,
                "{label}"
            );
            assert_eq!(stats.sent.load(Ordering::SeqCst), 1, "{label}");
        }
    }

    #[tokio::test]
    async fn assigned_solver_uses_successful_direct_artifact_without_solver_post() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/grab"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"<nzb></nzb>"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let stats = Arc::new(CountingStats {
            sent: AtomicU32::new(0),
            gate: None,
        });
        let transport = transport(Arc::clone(&stats));
        let now = Utc::now();
        let proxy = scryer_domain::ProxyConfig {
            id: "solver".into(),
            name: "fixture solver".into(),
            provider_type: scryer_domain::ProxyProviderType::Byparr,
            protocol: Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1),
            base_url: server.uri(),
            request_timeout_seconds: 5,
            is_enabled: true,
            username_encrypted: None,
            password_encrypted: None,
            remote_dns: false,
            private_key_encrypted: None,
            private_key_passphrase_encrypted: None,
            peer_public_key: None,
            preshared_key_encrypted: None,
            tunnel_public_key: None,
            tunnel_addresses: Vec::new(),
            tunnel_dns_servers: Vec::new(),
            tunnel_mtu: None,
            tunnel_keepalive_seconds: None,
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
        };
        let tally = IndexerRequestTally::new(
            "id".into(),
            "fixture".into(),
            IndexerErrorOperation::IndexerAction,
            stats.clone(),
        );
        let policy = PluginEgressPolicy::for_operator_configured_url(&server.uri());
        let artifact = transport
            .resolve_via_solver(
                &proxy,
                &format!("{}/grab", server.uri()),
                None,
                &policy,
                &server.uri(),
                DestinationKey::from("fixture"),
                &tally,
            )
            .await
            .expect("direct artifact");
        assert!(matches!(artifact, ResolvedDownloadArtifact::Nzb { .. }));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert_eq!(stats.sent.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn solver_post_counts_as_an_indexer_request_alongside_direct_grabs() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/grab"))
            .respond_with(ChallengeThenArtifact(AtomicU32::new(0)))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ok",
                "solution": { "status": 500, "cookies": ["clearance=x"] }
            })))
            .mount(&server)
            .await;
        let stats = Arc::new(CountingStats {
            sent: AtomicU32::new(0),
            gate: None,
        });
        let transport = transport(Arc::clone(&stats));
        let now = Utc::now();
        let proxy = scryer_domain::ProxyConfig {
            id: "solver".into(),
            name: "fixture solver".into(),
            provider_type: scryer_domain::ProxyProviderType::Byparr,
            protocol: Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1),
            base_url: server.uri(),
            request_timeout_seconds: 5,
            is_enabled: true,
            username_encrypted: None,
            password_encrypted: None,
            remote_dns: false,
            private_key_encrypted: None,
            private_key_passphrase_encrypted: None,
            peer_public_key: None,
            preshared_key_encrypted: None,
            tunnel_public_key: None,
            tunnel_addresses: Vec::new(),
            tunnel_dns_servers: Vec::new(),
            tunnel_mtu: None,
            tunnel_keepalive_seconds: None,
            host_key_fingerprint: None,
            host_key_pinned_at: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
        };
        let tally = IndexerRequestTally::new(
            "id".into(),
            "fixture".into(),
            IndexerErrorOperation::IndexerAction,
            stats.clone(),
        );
        let policy = PluginEgressPolicy::for_operator_configured_url(&server.uri());
        let artifact = transport
            .resolve_via_solver(
                &proxy,
                &format!("{}/grab", server.uri()),
                None,
                &policy,
                &server.uri(),
                DestinationKey::from("fixture"),
                &tally,
            )
            .await
            .expect("solver retry resolves artifact");
        assert!(matches!(artifact, ResolvedDownloadArtifact::Nzb { .. }));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.url.path() == "/grab")
                .count(),
            2
        );
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.url.path() == "/v1")
                .count(),
            1
        );
        assert_eq!(
            stats.sent.load(Ordering::SeqCst),
            3,
            "two direct grabs plus one solver-relayed grab each spend indexer quota"
        );
    }

    #[test]
    fn artifact_classification_keeps_body_magnets_and_rejects_fake_torrents() {
        let magnet = "magnet:?xt=urn:btih:0123456789012345678901234567890123456789";
        let artifact = classify(
            "fixture",
            Some("https://example.invalid/get"),
            None,
            magnet.as_bytes().to_vec(),
            None,
        )
        .unwrap();
        assert!(matches!(artifact, ResolvedDownloadArtifact::Magnet { uri, .. } if uri == magnet));
        assert!(
            classify(
                "fixture",
                Some("https://example.invalid/file.torrent"),
                None,
                b"<html>Login required</html>".to_vec(),
                None
            )
            .is_err()
        );
    }

    #[test]
    fn newznab_grab_quota_errors_are_typed() {
        for code in [500, 501] {
            let error = classify(
                "fixture",
                None,
                None,
                format!(r#"<?xml version="1.0"?><error code="{code}"/>"#).into_bytes(),
                None,
            )
            .expect_err("quota error must not be treated as an artifact");
            assert!(
                matches!(error, AppError::NewznabQuotaExceeded { code: actual, .. } if actual == code)
            );
        }
    }

    #[test]
    fn newznab_api_disabled_is_a_hard_error() {
        let error = classify(
            "fixture",
            None,
            None,
            br#"<?xml version="1.0"?><error code="910"/>"#.to_vec(),
            None,
        )
        .expect_err("disabled API must not be treated as an artifact");
        assert!(matches!(error, AppError::Validation(message) if message.contains("910")));
    }

    #[test]
    fn torrent_classification_keeps_validated_v1_info_hash() {
        let bytes = b"d4:infod3:foo3:baree".to_vec();
        let artifact = classify(
            "fixture",
            Some("https://indexer.test/file.torrent"),
            None,
            bytes,
            None,
        )
        .expect("valid torrent must resolve");
        let ResolvedDownloadArtifact::TorrentFile { info_hash_hint, .. } = artifact else {
            panic!("expected torrent artifact");
        };
        assert_eq!(
            info_hash_hint.as_deref(),
            Some("6d2262126feb6ec7bd3464935025c8c609c0119d")
        );
    }

    #[test]
    fn torrent_classification_hashes_pure_v2_and_preserves_hybrid_v1_identity() {
        for (info, expected) in [
            (
                "d9:file treed10:Titlea.bind0:d6:lengthi0eeee12:meta versioni2e4:name6:Titlea12:piece lengthi16384ee",
                "9049909a59fbd17feba71152334e41bbe9dcb75d3421130d0de9dcedd7c2d011",
            ),
            (
                "d6:lengthi0e12:meta versioni2e4:name6:Titlea12:piece lengthi16384e6:pieces0:e",
                "e72f7c79ef4428064ae525a67227bddd68d60e83",
            ),
        ] {
            let bytes = format!("d4:info{info}e").into_bytes();
            let artifact = classify("fixture", None, None, bytes, None).unwrap();
            let ResolvedDownloadArtifact::TorrentFile { info_hash_hint, .. } = artifact else {
                panic!("expected torrent");
            };
            assert_eq!(info_hash_hint.as_deref(), Some(expected));
        }
        // A similarly named nested field or opaque string is not the version.
        assert_eq!(
            super::torrent_hash(b"d4:infod4:metad12:meta versioni2eeee")
                .unwrap()
                .len(),
            40
        );
    }

    #[test]
    fn torrent_classification_rejects_noncanonical_integers() {
        for integer in ["", "abc", "-", "+1", "01", "-01", "-0", "1.0", "1 2", " 1"] {
            for value in [format!("i{integer}e"), format!("li{integer}ee")] {
                let bytes = format!("d4:infod6:length{value}ee").into_bytes();
                assert!(
                    classify("fixture", None, None, bytes, None).is_err(),
                    "{integer}"
                );
            }
        }
        // Bencode integers have no fixed-width limit; their lexical form does.
        for integer in ["0", "1", "-1", "999999999999999999999999999999999999"] {
            let bytes = format!("d4:infod6:lengthi{integer}eee").into_bytes();
            assert!(super::torrent_hash(&bytes).is_some(), "{integer}");
        }
    }

    struct ResolvingIndexerClient {
        calls: std::sync::Mutex<Vec<String>>,
        artifact: Option<ResolvedDownloadArtifact>,
    }

    #[async_trait]
    impl scryer_application::IndexerClient for ResolvingIndexerClient {
        async fn search(
            &self,
            _query: String,
            _ids: std::collections::HashMap<String, String>,
            _category: Option<String>,
            _facet: Option<String>,
            _id_search_facet: Option<String>,
            _newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<scryer_application::IndexerRoutingPlan>,
            _mode: scryer_application::SearchMode,
            _operation: scryer_application::IndexerErrorOperation,
            _season: Option<u32>,
            _episode: Option<u32>,
            _absolute_episode: Option<u32>,
            _year: Option<i32>,
            _tagged_aliases: Vec<scryer_domain::TaggedAlias>,
            _learning_context: Option<scryer_application::IndexerSearchLearningContext>,
            _cancel_token: CancellationToken,
        ) -> AppResult<scryer_application::IndexerSearchResponse> {
            Err(AppError::Repository(
                "search is not used in this test".to_string(),
            ))
        }

        async fn resolve_download(
            &self,
            download_url: &str,
        ) -> AppResult<Option<ResolvedDownloadArtifact>> {
            self.calls.lock().unwrap().push(download_url.to_string());
            Ok(self.artifact.clone())
        }
    }

    struct ResolvingIndexerProvider {
        client: Arc<dyn scryer_application::IndexerClient>,
    }

    impl IndexerPluginProvider for ResolvingIndexerProvider {
        fn client_for_provider(
            &self,
            _config: &IndexerConfig,
        ) -> Option<Arc<dyn scryer_application::IndexerClient>> {
            Some(Arc::clone(&self.client))
        }

        fn available_provider_types(&self) -> Vec<String> {
            vec!["cardigann".to_string()]
        }

        fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
            vec![]
        }
    }

    #[tokio::test]
    async fn indexer_owned_download_resolution_runs_before_direct_fallback() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let client = Arc::new(ResolvingIndexerClient {
            calls: std::sync::Mutex::new(Vec::new()),
            artifact: Some(ResolvedDownloadArtifact::TorrentFile {
                bytes: b"torrent bytes".to_vec(),
                file_name: Some("release.torrent".to_string()),
                content_type: Some("application/x-bittorrent".to_string()),
                info_hash_hint: Some("0123456789012345678901234567890123456789".to_string()),
            }),
        });
        let transport = transport_with_config(
            Arc::new(CountingStats {
                sent: AtomicU32::new(0),
                gate: None,
            }),
            test_config(&server.uri(), None),
        )
        .with_indexer_plugin_provider(Arc::new(ResolvingIndexerProvider {
            client: client.clone(),
        }));
        let download_url = format!("{}/download/1", server.uri());

        let response = transport
            .fetch_response(Some("indexer"), &download_url)
            .await
            .expect("indexer-owned resolution should avoid the direct fallback");

        assert_eq!(
            client.calls.lock().unwrap().as_slice(),
            &[download_url.clone()]
        );
        assert!(matches!(
            response,
            Some(ArtifactFetchResponse::Resolved(ResolvedDownloadArtifact::TorrentFile {
                ref bytes,
                ..
            })) if bytes == b"torrent bytes"
        ));
    }

    #[tokio::test]
    async fn indexers_without_a_grab_flow_fall_through_to_the_direct_fetch() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/x-nzb")
                    .set_body_bytes(b"<nzb/>".to_vec()),
            )
            .expect(1)
            .mount(&server)
            .await;
        let client = Arc::new(ResolvingIndexerClient {
            calls: std::sync::Mutex::new(Vec::new()),
            artifact: None,
        });
        let transport = transport_with_config(
            Arc::new(CountingStats {
                sent: AtomicU32::new(0),
                gate: None,
            }),
            test_config(&server.uri(), None),
        )
        .with_indexer_plugin_provider(Arc::new(ResolvingIndexerProvider {
            client: client.clone(),
        }));
        let download_url = format!("{}/download/2", server.uri());

        let response = transport
            .fetch_response(Some("indexer"), &download_url)
            .await
            .expect("a `None` grab answer keeps the direct fetch");

        assert_eq!(client.calls.lock().unwrap().len(), 1);
        assert!(matches!(response, Some(ArtifactFetchResponse::Http(_))));
    }
}
