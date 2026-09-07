use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use scryer_application::challenge_solver as solver;
use scryer_application::{
    AppError, AppResult, CONNECTION_TEST_INDEXER_ID, CapturedIndexerHttpResponse,
    IndexerConfigRepository, IndexerErrorOperation, IndexerProxyConfigRepository,
    IndexerRequestResult, IndexerRequestTally, IndexerStatsTracker, NullIndexerStatsTracker,
    RateLimitCooldownAction, ResolvedDownloadArtifact, extract_magnet_info_hash,
    is_valid_magnet_uri, normalize_torrent_info_hash,
};
use scryer_domain::IndexerConfig;
use scryer_outbound_http::{
    DestinationKey, OutboundHttpClient, OutboundHttpError, PluginEgressPolicy, RateLimitRegistry,
    RequestPolicy, generic_reqwest_client, indexer_proxy_reqwest_client,
    prepare_plugin_http_target, prepare_plugin_http_target_from_url,
};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

const ARTIFACT_MAX_BYTES: usize = 32 * 1024 * 1024;
const SOLVER_RESPONSE_MAX_BYTES: usize = ARTIFACT_MAX_BYTES * 2;

/// A successful artifact HTTP response whose body has not been buffered.
pub struct ArtifactHttpResponse {
    pub response: reqwest::Response,
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
    indexer_configs: Arc<dyn IndexerConfigRepository>,
    proxy_configs: Arc<dyn IndexerProxyConfigRepository>,
    indexer_stats: Arc<dyn IndexerStatsTracker>,
}

impl IndexerArtifactTransport {
    pub fn new(
        indexer_configs: Arc<dyn IndexerConfigRepository>,
        proxy_configs: Arc<dyn IndexerProxyConfigRepository>,
    ) -> Self {
        Self {
            outbound_http: OutboundHttpClient::new(
                generic_reqwest_client(),
                RateLimitRegistry::new(),
            ),
            indexer_configs,
            proxy_configs,
            indexer_stats: Arc::new(NullIndexerStatsTracker),
        }
    }

    pub fn with_indexer_stats_tracker(mut self, stats: Arc<dyn IndexerStatsTracker>) -> Self {
        self.indexer_stats = stats;
        self
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
                        &tally,
                    )
                    .await
                    .map(Some);
            }
        };
        if config.indexer_proxy_config_id.is_some()
            && !config.is_prowlarr_nab_proxy()
            && !config.provider_type.trim().eq_ignore_ascii_case("prowlarr")
        {
            return Ok(None);
        }
        let policy = PluginEgressPolicy::for_operator_configured_url(&config.base_url);
        let destination_cooldown_key = DestinationKey::from(config.rate_limit_domain_key());
        let tally = self.tally(Some(&config));
        self.fetch_response_direct(
            &config.name,
            download_url,
            scryer_outbound_http::STANDARD_HTTP_TIMEOUT,
            &policy,
            Some(&config.base_url),
            Some(destination_cooldown_key),
            &tally,
        )
        .await
        .map(Some)
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
                    )
                    .await;
            }
        };
        let policy = PluginEgressPolicy::for_operator_configured_url(&config.base_url);
        let destination_cooldown_key = DestinationKey::from(config.rate_limit_domain_key());
        let tally = self.tally(Some(&config));
        // Prowlarr owns challenge handling for its parent and synchronized children.
        if config.is_prowlarr_nab_proxy()
            || config.provider_type.trim().eq_ignore_ascii_case("prowlarr")
        {
            return self
                .fetch_and_classify(
                    &config.name,
                    download_url,
                    &[],
                    scryer_outbound_http::STANDARD_HTTP_TIMEOUT,
                    &policy,
                    Some(&config.base_url),
                    Some(destination_cooldown_key.clone()),
                    &tally,
                    info_hash_hint,
                )
                .await;
        }
        let Some(proxy_id) = config.indexer_proxy_config_id.as_deref() else {
            return self
                .fetch_and_classify(
                    &config.name,
                    download_url,
                    &[],
                    scryer_outbound_http::STANDARD_HTTP_TIMEOUT,
                    &policy,
                    Some(&config.base_url),
                    Some(destination_cooldown_key.clone()),
                    &tally,
                    info_hash_hint,
                )
                .await;
        };
        if !same_origin(&config, download_url) {
            return Err(AppError::Validation(
                "Proxied download URL does not match the assigned indexer origin.".into(),
            ));
        }
        let proxy = self
            .proxy_configs
            .get_by_id(proxy_id)
            .await?
            .ok_or_else(|| {
                AppError::DownloadSubmitUnavailable(
                    "The assigned indexer solver is no longer available.".into(),
                )
            })?;
        if !proxy.is_enabled {
            return Err(AppError::DownloadSubmitUnavailable(
                "The assigned indexer solver is disabled.".into(),
            ));
        }
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

    #[expect(
        clippy::too_many_arguments,
        reason = "preserve independent egress, deadline, and accounting policy through solver resolution"
    )]
    async fn resolve_via_solver(
        &self,
        proxy: &scryer_domain::IndexerProxyConfig,
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
        let timeout = scryer_outbound_http::effective_indexer_proxy_request_timeout(
            proxy.request_timeout_seconds,
        );
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
        let solver_http =
            OutboundHttpClient::new(indexer_proxy_reqwest_client(), RateLimitRegistry::new());
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
                    tally
                        .try_note_auxiliary_sent_call(scryer_application::CALL_SOLVER)
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
                AppError::DownloadSubmitUnavailable("The download artifact fetch timed out.".into())
            })?
            .map_err(|_| {
                AppError::DownloadSubmitUnavailable(
                    "Scryer refused an unsafe download artifact destination.".into(),
                )
            })?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(AppError::DownloadSubmitUnavailable(
                    "The download artifact fetch timed out.".into(),
                ));
            }
            let same = origin.0 == target.url().scheme()
                && origin
                    .1
                    .as_deref()
                    .zip(target.url().host_str())
                    .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
                && origin.2 == target.url().port_or_known_default();
            let mut builder = target.client().get(target.url().clone()).timeout(remaining);
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
            .map_err(|e| match e {
                OutboundHttpError::RateLimited(rate_limited) => AppError::TemporaryUnavailable {
                    message: "The download artifact destination is rate limited.".into(),
                    retry_after: rate_limited.retry_after,
                    rate_limit_cooldown: RateLimitCooldownAction::AlreadyRecorded,
                },
                OutboundHttpError::Transport { source, .. } if source.is_timeout() => {
                    tally.note_result(IndexerRequestResult::NoResponse);
                    AppError::DownloadSubmitUnavailable(
                        "The download artifact fetch timed out.".into(),
                    )
                }
                OutboundHttpError::Transport { .. } => {
                    tally.note_result(IndexerRequestResult::NoResponse);
                    AppError::DownloadSubmitUnavailable(format!(
                        "Scryer could not fetch the download artifact from '{name}'."
                    ))
                }
                _ => AppError::DownloadSubmitUnavailable(format!(
                    "Scryer could not fetch the download artifact from '{name}'."
                )),
            })?;
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
        tally: &IndexerRequestTally,
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
                AppError::DownloadSubmitUnavailable("The download artifact fetch timed out.".into())
            })?
            .map_err(|_| {
                AppError::DownloadSubmitUnavailable(
                    "Scryer refused an unsafe download artifact destination.".into(),
                )
            })?;
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
                        || target.client().get(target.url().clone()).timeout(remaining),
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
            .map_err(|error| map_artifact_outbound_error(name, tally, error))?;
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
                final_url,
                headers,
            }));
        }
        Err(AppError::Validation(
            "The download artifact redirect loop was exhausted.".into(),
        ))
    }
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
        let info_hash_hint = if hint
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
    let digest = aws_lc_rs::digest::digest(
        &aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY,
        &bytes[start..end],
    );
    Some(digest.as_ref().iter().map(|b| format!("{b:02x}")).collect())
}

fn parse_bencode_value(bytes: &[u8], offset: usize, depth: usize) -> Result<usize, ()> {
    if depth > 64 || offset >= bytes.len() {
        return Err(());
    }
    match bytes[offset] {
        b'i' => bytes[offset + 1..]
            .iter()
            .position(|b| *b == b'e')
            .map(|p| offset + p + 2)
            .ok_or(()),
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

    fn transport(stats: Arc<CountingStats>) -> IndexerArtifactTransport {
        IndexerArtifactTransport::new(
            Arc::new(TestConfigRepository::default()),
            Arc::new(scryer_application::NullIndexerProxyConfigRepository),
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
            indexer_proxy_config_id: proxy_id.map(str::to_string),
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
            Arc::new(scryer_application::NullIndexerProxyConfigRepository),
        )
        .with_indexer_stats_tracker(stats)
    }

    fn transport_with_config_and_proxy(
        stats: Arc<CountingStats>,
        config: IndexerConfig,
        proxy: scryer_domain::IndexerProxyConfig,
    ) -> IndexerArtifactTransport {
        IndexerArtifactTransport::new(
            Arc::new(TestConfigRepository {
                configs: vec![config],
            }),
            Arc::new(SingleProxyRepository(proxy)),
        )
        .with_indexer_stats_tracker(stats)
    }

    fn test_proxy(base_url: &str, is_enabled: bool) -> scryer_domain::IndexerProxyConfig {
        let now = Utc::now();
        scryer_domain::IndexerProxyConfig {
            id: "solver".into(),
            name: "fixture solver".into(),
            provider_type: scryer_domain::IndexerProxyProviderType::Byparr,
            protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
            base_url: base_url.into(),
            request_timeout_seconds: 5,
            is_enabled,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    struct SingleProxyRepository(scryer_domain::IndexerProxyConfig);

    #[async_trait]
    impl IndexerProxyConfigRepository for SingleProxyRepository {
        async fn list(
            &self,
            _: Option<scryer_domain::IndexerProxyProviderType>,
        ) -> AppResult<Vec<scryer_domain::IndexerProxyConfig>> {
            Ok(vec![self.0.clone()])
        }
        async fn get_by_id(
            &self,
            id: &str,
        ) -> AppResult<Option<scryer_domain::IndexerProxyConfig>> {
            Ok((id == self.0.id).then(|| self.0.clone()))
        }
        async fn create(
            &self,
            config: scryer_domain::IndexerProxyConfig,
        ) -> AppResult<scryer_domain::IndexerProxyConfig> {
            Ok(config)
        }
        async fn update(
            &self,
            config: scryer_domain::IndexerProxyConfig,
        ) -> AppResult<scryer_domain::IndexerProxyConfig> {
            Ok(config)
        }
        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
        async fn record_health(
            &self,
            _: &str,
            _: scryer_domain::IndexerProxyHealthStatus,
            _: Option<String>,
            _: Option<chrono::DateTime<Utc>>,
        ) -> AppResult<()> {
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
        let stats = Arc::new(CountingStats::default());
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
        assert!(error.to_string().contains("solver is no longer available"));
        assert!(server.received_requests().await.unwrap().is_empty());
        assert_eq!(stats.sent.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn metadata_destination_is_rejected_before_dispatch() {
        let stats = Arc::new(CountingStats::default());
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
        let stats = Arc::new(CountingStats::default());
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
        assert!(error.to_string().contains("solver is disabled"));
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
        let stats = Arc::new(CountingStats::default());
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
        let stats = Arc::new(CountingStats::default());
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
                &tally,
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
                &tally,
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
        let stats = Arc::new(CountingStats::default());
        let transport = transport(Arc::clone(&stats));
        let tally = IndexerRequestTally::new(
            "id".into(),
            "fixture".into(),
            IndexerErrorOperation::IndexerAction,
            stats,
        );
        let resolved = transport
            .fetch_response_direct(
                "fixture",
                &format!("{}/magnet", server.uri()),
                Duration::from_secs(5),
                &PluginEgressPolicy::default(),
                None,
                None,
                &tally,
            )
            .await
            .expect("magnet redirect");
        assert!(matches!(
            resolved,
            ArtifactFetchResponse::Resolved(ResolvedDownloadArtifact::Magnet { .. })
        ));
        let loop_result = transport
            .fetch_response_direct(
                "fixture",
                &format!("{}/a", server.uri()),
                Duration::from_secs(5),
                &PluginEgressPolicy::default(),
                None,
                None,
                &tally,
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
        let stats = Arc::new(CountingStats::default());
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
        let stats = Arc::new(CountingStats::default());
        let transport = transport(Arc::clone(&stats));
        let now = Utc::now();
        let proxy = scryer_domain::IndexerProxyConfig {
            id: "solver".into(),
            name: "fixture solver".into(),
            provider_type: scryer_domain::IndexerProxyProviderType::Byparr,
            protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
            base_url: server.uri(),
            request_timeout_seconds: 5,
            is_enabled: true,
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
            let stats = Arc::new(CountingStats::default());
            let transport = transport(Arc::clone(&stats));
            let now = Utc::now();
            let proxy = scryer_domain::IndexerProxyConfig {
                id: "solver".into(),
                name: "fixture solver".into(),
                provider_type: scryer_domain::IndexerProxyProviderType::Byparr,
                protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
                base_url: server.uri(),
                request_timeout_seconds: 5,
                is_enabled: true,
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
        let stats = Arc::new(CountingStats::default());
        let transport = transport(Arc::clone(&stats));
        let now = Utc::now();
        let proxy = scryer_domain::IndexerProxyConfig {
            id: "solver".into(),
            name: "fixture solver".into(),
            provider_type: scryer_domain::IndexerProxyProviderType::Byparr,
            protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
            base_url: server.uri(),
            request_timeout_seconds: 5,
            is_enabled: true,
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
    async fn solver_post_is_auxiliary_and_retry_counts_only_indexer_grabs() {
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
        let stats = Arc::new(CountingStats::default());
        let transport = transport(Arc::clone(&stats));
        let now = Utc::now();
        let proxy = scryer_domain::IndexerProxyConfig {
            id: "solver".into(),
            name: "fixture solver".into(),
            provider_type: scryer_domain::IndexerProxyProviderType::Byparr,
            protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
            base_url: server.uri(),
            request_timeout_seconds: 5,
            is_enabled: true,
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
            2,
            "solver POST must not increment indexer request quota"
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
}
