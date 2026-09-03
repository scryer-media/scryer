use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::sync::{Arc, LazyLock, Mutex, mpsc};
use std::time::{Duration, Instant};

use glob::Pattern;
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use reqwest::blocking::Client;
use reqwest::{Method, StatusCode};
use scryer_application::{
    CapturedIndexerHttpHeader, CapturedIndexerHttpResponse, IndexerErrorOperation,
    IndexerErrorRecorder, challenge_solver as solver, classify_indexer_http_response,
    indexer_response_content_type, unknown_indexer_error,
};

pub(crate) const HTTP_ENV_NAMESPACE: &str = "extism:host/env";
const DEFAULT_MAX_HTTP_RESPONSE_BYTES: u64 = 50 * 1024 * 1024;
const PLUGIN_HTTP_WORKER_RESPONSE_GRACE: Duration = Duration::from_secs(1);
const PINNED_REQUEST_CLIENT_TTL: Duration = Duration::from_secs(5 * 60);
type HostResult<T> = Result<T, String>;

static SHARED_PLUGIN_HTTP_RUNTIME: LazyLock<PluginHttpRuntime> =
    LazyLock::new(PluginHttpRuntime::default);

#[derive(Clone, Default)]
pub struct PluginHttpRuntime {
    state: Arc<Mutex<PluginHttpRuntimeState>>,
}

#[derive(Default)]
struct PluginHttpRuntimeState {
    extra_ca_bundle_pem: String,
}

#[derive(Clone)]
pub(crate) struct PluginHttpHost {
    state: Arc<Mutex<PluginHttpHostState>>,
    workers: Arc<Mutex<HashMap<PluginHttpRequestClientKey, mpsc::SyncSender<PluginHttpWork>>>>,
}

struct PluginHttpHostState {
    runtime: PluginHttpRuntime,
    allowed_hosts: Option<Vec<String>>,
    egress_policy: scryer_outbound_http::PluginEgressPolicy,
    indexer_proxy_policy: Option<IndexerProxyPolicy>,
    destination_cooldown_key: Option<scryer_outbound_http::DestinationKey>,
    max_http_response_bytes: Option<u64>,
    last_responses: HashMap<String, PluginHttpLastResponse>,
    indexer_error_capture: Option<ActiveIndexerErrorCapture>,
}

#[derive(Clone)]
pub(crate) struct IndexerErrorCaptureContext {
    pub(crate) indexer_id: String,
    pub(crate) indexer_name: String,
    pub(crate) operation: IndexerErrorOperation,
    pub(crate) recorder: Arc<dyn IndexerErrorRecorder>,
}

struct ActiveIndexerErrorCapture {
    context: IndexerErrorCaptureContext,
    final_response: Option<CapturedIndexerHttpResponse>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct PluginHttpRequest {
    pub(crate) url: String,
    #[serde(default)]
    pub(crate) method: Option<String>,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
}

#[derive(Clone, Default)]
struct PluginHttpLastResponse {
    status_code: u16,
    headers: BTreeMap<String, String>,
    rate_limit_message: Option<String>,
}

struct PluginHttpWork {
    plugin_id: String,
    request: PluginHttpRequest,
    body: Option<Vec<u8>>,
    timeout: Duration,
    response: mpsc::SyncSender<HostResult<Vec<u8>>>,
}

#[derive(Default)]
struct PluginHttpWorkerRuntime {
    extra_ca_bundle_pem: String,
    request_clients: HashMap<PluginHttpRequestClientKey, CachedPluginHttpClient>,
    proxy_client: Option<Client>,
}

struct CachedPluginHttpClient {
    client: Client,
    created_at: Instant,
}

impl CachedPluginHttpClient {
    fn is_fresh(&self) -> bool {
        pinned_client_is_fresh(self.created_at)
    }
}

fn pinned_client_is_fresh(created_at: Instant) -> bool {
    created_at.elapsed() < PINNED_REQUEST_CLIENT_TTL
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct PluginHttpRequestClientKey {
    scheme: String,
    host: String,
    port: u16,
}

#[derive(Clone)]
pub(crate) struct IndexerProxyPolicy {
    pub indexer_id: String,
    pub indexer_name: String,
    pub config: scryer_domain::IndexerProxyConfig,
}

struct ProxiedHttpResponse {
    status_code: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
    captured_response: CapturedIndexerHttpResponse,
    terminal_error: Option<String>,
}

pub fn shared_plugin_http_runtime() -> PluginHttpRuntime {
    SHARED_PLUGIN_HTTP_RUNTIME.clone()
}

impl scryer_application::PluginHttpTrustConfigRuntime for PluginHttpRuntime {
    fn set_plugin_http_ca_bundle_pem(
        &self,
        bundle_pem: String,
    ) -> scryer_application::AppResult<()> {
        self.set_extra_ca_bundle_pem(bundle_pem)
            .map_err(scryer_application::AppError::Repository)
    }
}

impl PluginHttpRuntime {
    pub fn set_extra_ca_bundle_pem(&self, bundle_pem: impl Into<String>) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|error| format!("plugin HTTP runtime lock poisoned: {error}"))?;
        state.extra_ca_bundle_pem = bundle_pem.into();
        Ok(())
    }

    pub(crate) fn extra_ca_bundle_pem(&self) -> HostResult<String> {
        self.state
            .lock()
            .map_err(|error| format!("plugin HTTP runtime lock poisoned: {error}"))
            .map(|state| state.extra_ca_bundle_pem.clone())
    }
}

impl PluginHttpWorkerRuntime {
    fn sync_trust_bundle(&mut self, runtime: &PluginHttpRuntime) -> HostResult<String> {
        let extra_ca_bundle_pem = runtime.extra_ca_bundle_pem()?;
        if self.extra_ca_bundle_pem != extra_ca_bundle_pem {
            self.extra_ca_bundle_pem = extra_ca_bundle_pem.clone();
            // These values only ever live on the HTTP worker, so replacing a
            // trust bundle also drops every old client off the async runtime.
            self.request_clients.clear();
            self.proxy_client = None;
        }
        Ok(extra_ca_bundle_pem)
    }

    fn pinned_request_client(
        &mut self,
        runtime: &PluginHttpRuntime,
        request_url: &str,
        egress_policy: &scryer_outbound_http::PluginEgressPolicy,
    ) -> HostResult<Client> {
        let key = plugin_http_request_client_key(request_url)?;
        let extra_ca_bundle_pem = self.sync_trust_bundle(runtime)?;
        if let Some(cached) = self.request_clients.get(&key)
            && cached.is_fresh()
        {
            return Ok(cached.client.clone());
        }

        let client = scryer_outbound_http::prepare_plugin_blocking_http_target_with_policy(
            request_url,
            &extra_ca_bundle_pem,
            "plugin HTTP",
            egress_policy,
        )
        .map(scryer_outbound_http::PinnedPluginBlockingHttpTarget::into_client)
        .map_err(|error| error.to_string())?;
        self.request_clients.insert(
            key,
            CachedPluginHttpClient {
                client: client.clone(),
                created_at: Instant::now(),
            },
        );
        Ok(client)
    }

    fn proxy_client(&mut self, runtime: &PluginHttpRuntime) -> HostResult<Client> {
        let extra_ca_bundle_pem = self.sync_trust_bundle(runtime)?;
        if let Some(client) = &self.proxy_client {
            return Ok(client.clone());
        }

        let client =
            scryer_outbound_http::blocking_indexer_proxy_reqwest_client(&extra_ca_bundle_pem)
                .map_err(|error| error.to_string())?;
        self.proxy_client = Some(client.clone());
        Ok(client)
    }
}

fn plugin_http_request_client_key(request_url: &str) -> HostResult<PluginHttpRequestClientKey> {
    let url = url::Url::parse(request_url).map_err(|error| format!("Invalid URL: {error:?}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("Invalid URL scheme: {}", url.scheme()));
    }
    let host = url
        .host_str()
        .ok_or_else(|| "Invalid URL: missing host".to_string())?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "Invalid URL: missing port".to_string())?;
    Ok(PluginHttpRequestClientKey {
        scheme: url.scheme().to_string(),
        host: host.to_ascii_lowercase(),
        port,
    })
}

fn worker_response_timeout(timeout: Duration) -> Duration {
    timeout.saturating_add(PLUGIN_HTTP_WORKER_RESPONSE_GRACE)
}

impl PluginHttpHost {
    pub(crate) fn new(
        allowed_hosts: Vec<String>,
        indexer_proxy_policy: Option<IndexerProxyPolicy>,
        destination_cooldown_key: Option<String>,
        max_http_response_bytes: Option<u64>,
    ) -> Self {
        Self::new_with_egress_policy(
            allowed_hosts,
            scryer_outbound_http::PluginEgressPolicy::default(),
            indexer_proxy_policy,
            destination_cooldown_key,
            max_http_response_bytes,
        )
    }

    pub(crate) fn new_with_egress_policy(
        allowed_hosts: Vec<String>,
        egress_policy: scryer_outbound_http::PluginEgressPolicy,
        indexer_proxy_policy: Option<IndexerProxyPolicy>,
        destination_cooldown_key: Option<String>,
        max_http_response_bytes: Option<u64>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(PluginHttpHostState {
                runtime: shared_plugin_http_runtime(),
                allowed_hosts: Some(allowed_hosts),
                egress_policy,
                indexer_proxy_policy,
                destination_cooldown_key: destination_cooldown_key
                    .map(scryer_outbound_http::DestinationKey::from),
                max_http_response_bytes,
                last_responses: HashMap::new(),
                indexer_error_capture: None,
            })),
            workers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) fn request(
        &self,
        plugin_id: &str,
        request: PluginHttpRequest,
        body: Option<Vec<u8>>,
        timeout: Duration,
    ) -> HostResult<Vec<u8>> {
        let worker_key = plugin_http_request_client_key(&request.url)?;
        let allowed_hosts = self
            .state
            .lock()
            .map_err(|error| format!("plugin HTTP host state lock poisoned: {error}"))?
            .allowed_hosts
            .clone();
        enforce_allowed_hosts(allowed_hosts.as_deref(), &request.url)?;
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        let worker = self.worker_sender(worker_key.clone())?;
        worker
            .send(PluginHttpWork {
                plugin_id: plugin_id.to_string(),
                request,
                body,
                timeout,
                response: response_sender,
            })
            .map_err(|_| {
                self.reset_worker(&worker_key);
                "plugin HTTP worker stopped".to_string()
            })?;
        let response_timeout = worker_response_timeout(timeout);
        match response_receiver.recv_timeout(response_timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // DNS resolution happens while the worker builds a pinned
                // client, before reqwest's request timeout can apply. Drop the
                // sender mapping so later requests get a fresh worker instead
                // of queueing behind that stalled resolution.
                self.reset_worker(&worker_key);
                Err(format!(
                    "plugin HTTP worker timed out after {} ms",
                    response_timeout.as_millis()
                ))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.reset_worker(&worker_key);
                Err("plugin HTTP worker stopped".to_string())
            }
        }
    }

    pub(crate) fn begin_indexer_error_capture(&self, context: IndexerErrorCaptureContext) {
        match self.state.lock() {
            Ok(mut state) => {
                state.indexer_error_capture = Some(ActiveIndexerErrorCapture {
                    context,
                    final_response: None,
                });
            }
            Err(error) => {
                tracing::warn!(error = %error, "failed to start indexer HTTP error capture")
            }
        }
    }

    pub(crate) fn finish_indexer_error_capture(&self, operation_failed: bool) {
        let capture = match self.state.lock() {
            Ok(mut state) => state.indexer_error_capture.take(),
            Err(error) => {
                tracing::warn!(error = %error, "failed to finish indexer HTTP error capture");
                None
            }
        };
        let Some(capture) = capture else {
            return;
        };
        let Some(response) = capture.final_response else {
            if operation_failed {
                Self::record_transport_failure(&capture.context);
            }
            return;
        };
        if operation_failed && (200..300).contains(&response.status) {
            Self::record_captured_response(&capture.context, response);
        }
    }

    fn worker_sender(
        &self,
        worker_key: PluginHttpRequestClientKey,
    ) -> HostResult<mpsc::SyncSender<PluginHttpWork>> {
        let mut workers = self
            .workers
            .lock()
            .map_err(|error| format!("plugin HTTP worker lock poisoned: {error}"))?;
        if let Some(sender) = workers.get(&worker_key) {
            return Ok(sender.clone());
        }

        let (sender, receiver) = mpsc::sync_channel::<PluginHttpWork>(32);
        let state = Arc::clone(&self.state);
        std::thread::Builder::new()
            .name("scryer-plugin-http".to_string())
            .spawn(move || {
                // This long-lived dispatcher owns the origin's pinned-client
                // cache. Request tasks receive Client clones, which share that
                // client's connection pool, so a slow request cannot block a
                // later request to the same indexer.
                let runtime = Arc::new(Mutex::new(PluginHttpWorkerRuntime::default()));
                let mut request_tasks: Vec<std::thread::JoinHandle<()>> = Vec::new();
                while let Ok(work) = receiver.recv() {
                    let mut still_running = Vec::new();
                    for task in request_tasks.drain(..) {
                        if task.is_finished() {
                            let _ = task.join();
                        } else {
                            still_running.push(task);
                        }
                    }
                    request_tasks = still_running;

                    let worker_state = Arc::clone(&state);
                    let worker_runtime = Arc::clone(&runtime);
                    let PluginHttpWork {
                        plugin_id,
                        request,
                        body,
                        timeout,
                        response,
                    } = work;
                    match std::thread::Builder::new()
                        .name("scryer-plugin-http-request".to_string())
                        .spawn(move || {
                            let result = Self::request_inner(
                                &worker_state,
                                &worker_runtime,
                                &plugin_id,
                                request,
                                body,
                                timeout,
                            );
                            let _ = response.send(result);
                        }) {
                        Ok(task) => request_tasks.push(task),
                        Err(error) => {
                            tracing::warn!(error = %error, "failed to start plugin HTTP request task");
                        }
                    }
                }
                // Keep the cached client owned by this dispatcher until all
                // request-client clones have been dropped off async runtimes.
                for task in request_tasks {
                    let _ = task.join();
                }
            })
            .map_err(|error| format!("failed to start plugin HTTP worker: {error}"))?;
        workers.insert(worker_key, sender.clone());
        Ok(sender)
    }

    fn reset_worker(&self, worker_key: &PluginHttpRequestClientKey) {
        if let Ok(mut workers) = self.workers.lock() {
            workers.remove(worker_key);
        }
    }

    fn request_inner(
        state: &Arc<Mutex<PluginHttpHostState>>,
        worker_runtime: &Arc<Mutex<PluginHttpWorkerRuntime>>,
        plugin_id: &str,
        request: PluginHttpRequest,
        body: Option<Vec<u8>>,
        timeout: Duration,
    ) -> HostResult<Vec<u8>> {
        let (
            runtime,
            allowed_hosts,
            egress_policy,
            indexer_proxy_policy,
            destination_cooldown_key,
            max_http_response_bytes,
        ) = {
            let mut host_state = state
                .lock()
                .map_err(|error| format!("plugin HTTP host state lock poisoned: {error}"))?;
            host_state
                .last_responses
                .insert(plugin_id.to_string(), PluginHttpLastResponse::default());
            (
                host_state.runtime.clone(),
                host_state.allowed_hosts.clone(),
                host_state.egress_policy.clone(),
                host_state.indexer_proxy_policy.clone(),
                host_state.destination_cooldown_key.clone(),
                host_state.max_http_response_bytes,
            )
        };

        enforce_allowed_hosts(allowed_hosts.as_deref(), &request.url)?;

        // The allowlist is the primary boundary; the guarded, DNS-pinned client
        // is the second layer that keeps a declared host from reaching
        // link-local / cloud-metadata space.
        let request_client = worker_runtime
            .lock()
            .map_err(|error| format!("plugin HTTP worker runtime lock poisoned: {error}"))?
            .pinned_request_client(&runtime, &request.url, &egress_policy)?;
        let started_at = Instant::now();
        let request_is_get = request
            .method
            .as_deref()
            .unwrap_or("GET")
            .eq_ignore_ascii_case("GET");
        // Reuse a previously solved clearance session for this proxy + origin so
        // repeat requests skip the solver entirely until the session goes stale.
        let session_headers = indexer_proxy_policy
            .as_ref()
            .filter(|_| request_is_get)
            .map(|policy| {
                solver::SolvedSessionCache::shared()
                    .session_headers(&policy.config.id, &request.url)
            })
            .unwrap_or_default();
        let response = execute_request_with_extra_headers(
            &request_client,
            &request,
            body.clone(),
            Some(timeout),
            &session_headers,
            destination_cooldown_key.as_ref(),
        )?;
        let status = response.status();
        let status_code = status.as_u16();
        let headers = response_headers(&response);
        let captured_headers = captured_response_headers(&response);
        let direct_body = read_response_body(response, max_http_response_bytes)?;
        Self::capture_indexer_response(
            state,
            CapturedIndexerHttpResponse {
                status: status_code,
                headers: captured_headers,
                body: direct_body.clone(),
            },
        );

        if status == StatusCode::TOO_MANY_REQUESTS {
            let response_bytes = direct_body.len();
            let rate_limit_message = direct_rate_limit_message(&headers, &direct_body);
            Self::store_last_response(state, plugin_id, status_code, headers)?;
            Self::store_rate_limit_message(state, plugin_id, rate_limit_message)?;
            tracing::debug!(
                plugin_id,
                status = status_code,
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                response_bytes,
                "plugin HTTP request skipped indexer proxy after direct rate limit"
            );
            return Ok(direct_body);
        }

        if let Some(policy) = indexer_proxy_policy.as_ref()
            && solver::looks_like_challenge_response(status_code, &headers, &direct_body)
        {
            let method = request.method.as_deref().unwrap_or("GET");
            if !method.eq_ignore_ascii_case("GET") {
                return Err(format!(
                    "indexer proxy only supports GET challenge solving for plugin HTTP requests; got {method}"
                ));
            }
            if !session_headers.is_empty() {
                // The cached session no longer clears the challenge.
                solver::SolvedSessionCache::shared().invalidate(&policy.config.id, &request.url);
            }

            tracing::debug!(
                plugin_id,
                indexer_id = policy.indexer_id.as_str(),
                indexer_name = policy.indexer_name.as_str(),
                proxy_config_id = policy.config.id.as_str(),
                status = status_code,
                request_url = solver::sanitized_url_for_log(&request.url).as_str(),
                "plugin HTTP request detected browser challenge"
            );

            // The proxy endpoint itself is operator-configured, so it uses the
            // trusted client; the plugin URL retry inside stays on the guarded
            // pinned client.
            let proxy_client = worker_runtime
                .lock()
                .map_err(|error| format!("plugin HTTP worker runtime lock poisoned: {error}"))?
                .proxy_client(&runtime)?;
            let solved = match execute_challenge_solver_request(
                &proxy_client,
                &request_client,
                policy,
                &request,
                ChallengeSolverRequestOptions {
                    original_body: body,
                    original_timeout: Some(timeout),
                    max_http_response_bytes,
                    destination_cooldown_key: destination_cooldown_key.as_ref(),
                },
            ) {
                Ok(solved) => {
                    solver::SolverHealthLedger::shared().record_success(&policy.config.id);
                    solved
                }
                Err(error) => {
                    if solver::is_solver_service_error_message(&error) {
                        solver::SolverHealthLedger::shared()
                            .record_failure(&policy.config.id, &error);
                    }
                    return Err(error);
                }
            };
            let response_bytes = solved.body.len();
            Self::capture_indexer_response(state, solved.captured_response.clone());
            Self::store_last_response(state, plugin_id, solved.status_code, solved.headers)?;
            tracing::debug!(
                plugin_id,
                indexer_id = policy.indexer_id.as_str(),
                proxy_config_id = policy.config.id.as_str(),
                status = solved.status_code,
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                response_bytes,
                "plugin HTTP request completed through indexer proxy"
            );
            if let Some(error) = solved.terminal_error {
                return Err(error);
            }
            return Ok(solved.body);
        }

        let response_bytes = direct_body.len();
        Self::store_last_response(state, plugin_id, status_code, headers)?;
        tracing::debug!(
            plugin_id,
            status = status_code,
            elapsed_ms = started_at.elapsed().as_millis() as u64,
            response_bytes,
            "plugin HTTP request completed"
        );
        if status.is_success() {
            Ok(direct_body)
        } else {
            Ok(Vec::new())
        }
    }

    pub(crate) fn status_code(&self, plugin_id: &str) -> HostResult<u16> {
        let host_state = self
            .state
            .lock()
            .map_err(|error| format!("plugin HTTP host state lock poisoned: {error}"))?;
        Ok(host_state
            .last_responses
            .get(plugin_id)
            .map(|response| response.status_code)
            .unwrap_or(0))
    }

    pub(crate) fn headers(&self, plugin_id: &str) -> HostResult<Option<BTreeMap<String, String>>> {
        let host_state = self
            .state
            .lock()
            .map_err(|error| format!("plugin HTTP host state lock poisoned: {error}"))?;
        Ok(host_state
            .last_responses
            .get(plugin_id)
            .filter(|response| !response.headers.is_empty())
            .map(|response| response.headers.clone()))
    }

    pub(crate) fn rate_limit_message(&self, plugin_id: &str) -> HostResult<Option<String>> {
        let host_state = self
            .state
            .lock()
            .map_err(|error| format!("plugin HTTP host state lock poisoned: {error}"))?;
        Ok(host_state
            .last_responses
            .get(plugin_id)
            .and_then(|response| response.rate_limit_message.clone()))
    }

    fn capture_indexer_response(
        state: &Arc<Mutex<PluginHttpHostState>>,
        response: CapturedIndexerHttpResponse,
    ) {
        let immediate_capture = match state.lock() {
            Ok(mut host_state) => {
                let Some(capture) = host_state.indexer_error_capture.as_mut() else {
                    return;
                };
                capture.final_response = Some(response.clone());
                (!(200..300).contains(&response.status)).then(|| capture.context.clone())
            }
            Err(error) => {
                tracing::warn!(error = %error, "failed to capture indexer HTTP response");
                None
            }
        };
        if let Some(capture) = immediate_capture {
            Self::record_captured_response(&capture, response);
        }
    }

    pub(crate) fn record_captured_response(
        capture: &IndexerErrorCaptureContext,
        response: CapturedIndexerHttpResponse,
    ) {
        if !scryer_application::indexer_error_history_is_persistable(&capture.indexer_id) {
            // A connection test has no stored indexer to hang history on. The
            // probe's own error is the answer the operator is waiting for; a
            // guaranteed foreign-key failure behind it is noise at best.
            tracing::debug!(
                indexer_id = capture.indexer_id.as_str(),
                "skipping indexer error history for a synthetic indexer id"
            );
            return;
        }

        let classified =
            classify_indexer_http_response(&response).unwrap_or_else(unknown_indexer_error);
        let error = scryer_application::NewIndexerError {
            id: uuid::Uuid::new_v4().to_string(),
            indexer_id: capture.indexer_id.clone(),
            indexer_name: capture.indexer_name.clone(),
            operation: capture.operation,
            classification: classified.classification,
            provider_error_code: classified.provider_error_code,
            message: classified.message.to_string(),
            content_type: indexer_response_content_type(&response),
            response: Some(response),
            occurred_at: chrono::Utc::now(),
        };
        if let Err(error) = capture.recorder.record(error) {
            tracing::warn!(
                indexer_id = capture.indexer_id.as_str(),
                error = %error,
                "failed to persist captured indexer HTTP response"
            );
        }
    }

    pub(crate) fn record_transport_failure(capture: &IndexerErrorCaptureContext) {
        tracing::warn!(
            indexer_id = capture.indexer_id.as_str(),
            operation = capture.operation.as_str(),
            "indexer plugin operation failed without an HTTP response"
        );
        if !scryer_application::indexer_error_history_is_persistable(&capture.indexer_id) {
            return;
        }
        let error = scryer_application::NewIndexerError {
            id: uuid::Uuid::new_v4().to_string(),
            indexer_id: capture.indexer_id.clone(),
            indexer_name: capture.indexer_name.clone(),
            operation: capture.operation,
            classification: scryer_application::IndexerErrorClassification::Unknown,
            provider_error_code: None,
            message: "Indexer plugin command failed without an HTTP response".to_string(),
            content_type: None,
            response: None,
            occurred_at: chrono::Utc::now(),
        };
        if let Err(error) = capture.recorder.record(error) {
            tracing::warn!(
                indexer_id = capture.indexer_id.as_str(),
                error = %error,
                "failed to persist indexer transport error"
            );
        }
    }

    fn store_rate_limit_message(
        state: &Arc<Mutex<PluginHttpHostState>>,
        plugin_id: &str,
        message: String,
    ) -> HostResult<()> {
        let mut host_state = state
            .lock()
            .map_err(|error| format!("plugin HTTP host state lock poisoned: {error}"))?;
        if let Some(response) = host_state.last_responses.get_mut(plugin_id) {
            response.rate_limit_message = Some(message);
        }
        Ok(())
    }

    fn store_last_response(
        state: &Arc<Mutex<PluginHttpHostState>>,
        plugin_id: &str,
        status_code: u16,
        headers: BTreeMap<String, String>,
    ) -> HostResult<()> {
        let mut host_state = state
            .lock()
            .map_err(|error| format!("plugin HTTP host state lock poisoned: {error}"))?;
        host_state.last_responses.insert(
            plugin_id.to_string(),
            PluginHttpLastResponse {
                status_code,
                headers,
                rate_limit_message: None,
            },
        );
        Ok(())
    }
}

pub(crate) fn enforce_allowed_hosts(
    allowed_hosts: Option<&[String]>,
    request_url: &str,
) -> HostResult<()> {
    let url = url::Url::parse(request_url).map_err(|error| format!("Invalid URL: {error:?}"))?;
    let host = url.host_str().unwrap_or_default();
    let matches = allowed_hosts.is_some_and(|patterns| {
        patterns.iter().any(|pattern| {
            Pattern::new(pattern)
                .map(|compiled| compiled.matches(host))
                .unwrap_or_else(|_| pattern == host)
        })
    });

    if matches {
        return Ok(());
    }

    // Never interpolate the raw request URL here: for indexer requests it
    // carries `?apikey=`/`passkey=` credentials that would otherwise leak into
    // WARN logs and the user-facing error. Log the query-stripped URL instead.
    Err(format!(
        "HTTP request to {} is not allowed",
        solver::sanitized_url_for_log(request_url)
    ))
}

fn merge_cookie_headers(original: Option<&str>, solved: &str) -> Option<String> {
    let mut cookies: Vec<(String, String)> = Vec::new();
    for header in original.into_iter().chain(std::iter::once(solved)) {
        for raw_pair in header.split(';') {
            let pair = raw_pair.trim();
            if !solver::safe_cookie_pair(pair) {
                continue;
            }
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            let name = name.trim().to_string();
            let value = value.trim().to_string();
            if let Some(existing) = cookies.iter_mut().find(|(existing, _)| existing == &name) {
                existing.1 = value;
            } else {
                cookies.push((name, value));
            }
        }
    }
    (!cookies.is_empty()).then(|| {
        cookies
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    })
}

fn execute_request_with_extra_headers(
    client: &Client,
    request: &PluginHttpRequest,
    body: Option<Vec<u8>>,
    timeout: Option<Duration>,
    extra_headers: &[(String, String)],
    destination_cooldown_key: Option<&scryer_outbound_http::DestinationKey>,
) -> HostResult<reqwest::blocking::Response> {
    let method = Method::from_bytes(
        request
            .method
            .as_deref()
            .unwrap_or("GET")
            .to_uppercase()
            .as_bytes(),
    )
    .map_err(|error| format!("Invalid HTTP method: {error}"))?;

    let solved_cookie = extra_headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("cookie"))
        .map(|(_, value)| value.as_str());
    let original_cookie = request
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("cookie"))
        .map(|(_, value)| value.as_str());
    let merged_cookie =
        solved_cookie.and_then(|solved| merge_cookie_headers(original_cookie, solved));

    let mut builder = client.request(method, &request.url);
    for (name, value) in &request.headers {
        // Solver-session headers replace matching plugin headers, except Cookie:
        // preserve unrelated indexer cookies and overlay solved cookie names.
        if extra_headers
            .iter()
            .any(|(extra, _)| extra.eq_ignore_ascii_case(name))
        {
            continue;
        }
        builder = builder.header(name, value);
    }
    for (name, value) in extra_headers {
        if name.eq_ignore_ascii_case("cookie") {
            if let Some(value) = merged_cookie.as_deref() {
                builder = builder.header(name, value);
            }
        } else {
            builder = builder.header(name, value);
        }
    }
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    if let Some(body) = body {
        builder = builder.body(body);
    }

    scryer_outbound_http::send_blocking_reqwest_request_with_cooldown_policy(
        builder,
        timeout,
        destination_cooldown_key.cloned(),
    )
    .map_err(|error| match error {
        scryer_outbound_http::BlockingOutboundHttpError::Request(error) if error.is_timeout() => {
            "timeout".to_string()
        }
        other => other.to_string(),
    })
}

fn response_headers(response: &reqwest::blocking::Response) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    for (name, value) in response.headers() {
        if let Ok(value) = value.to_str() {
            headers.insert(name.as_str().to_string(), value.to_string());
        }
    }
    headers
}

fn captured_response_headers(
    response: &reqwest::blocking::Response,
) -> Vec<CapturedIndexerHttpHeader> {
    response
        .headers()
        .iter()
        .map(|(name, value)| CapturedIndexerHttpHeader {
            name: name.as_str().to_string(),
            value: value.as_bytes().to_vec(),
        })
        .collect()
}

fn captured_headers_from_text(
    headers: &BTreeMap<String, String>,
) -> Vec<CapturedIndexerHttpHeader> {
    headers
        .iter()
        .map(|(name, value)| CapturedIndexerHttpHeader {
            name: name.clone(),
            value: value.as_bytes().to_vec(),
        })
        .collect()
}

fn direct_rate_limit_message(headers: &BTreeMap<String, String>, body: &[u8]) -> String {
    let mut message = prowlarr_xml_error_description(body)
        .map(|description| format!("HTTP 429: {description}"))
        .unwrap_or_else(|| "HTTP 429: rate limited".to_string());
    let retry_after = headers
        .get("retry-after")
        .and_then(|value| scryer_outbound_http::parse_retry_after(value))
        .map(|(delay, _)| delay);
    if let Some(retry_after) = retry_after {
        message.push_str(&format!("; retry after {}s", retry_after.as_secs()));
    }
    message
}

fn prowlarr_xml_error_description(body: &[u8]) -> Option<String> {
    // Prowlarr's NewznabController::CreateErrorXML emits an XML declaration followed by
    // one root element: <error code="429" description="..." />.
    let mut reader = Reader::from_reader(body);
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer).ok()? {
            Event::Start(element) | Event::Empty(element) => {
                if element.name().as_ref() != "error" {
                    return None;
                }

                let mut code = None;
                let mut description = None;
                for attribute in element.attributes() {
                    let attribute = attribute.ok()?;
                    let value = attribute
                        .normalized_value(XmlVersion::Explicit1_0)
                        .ok()?
                        .trim()
                        .to_string();
                    match attribute.key.as_ref() {
                        "code" => code = Some(value),
                        "description" => description = (!value.is_empty()).then_some(value),
                        _ => {}
                    }
                }

                return (code.as_deref() == Some("429"))
                    .then_some(description)
                    .flatten();
            }
            Event::Eof => return None,
            _ => {}
        }
        buffer.clear();
    }
}

fn read_response_body(
    response: reqwest::blocking::Response,
    max_http_response_bytes: Option<u64>,
) -> HostResult<Vec<u8>> {
    let mut body = Vec::new();
    let max = max_http_response_bytes.unwrap_or(DEFAULT_MAX_HTTP_RESPONSE_BYTES);
    response
        .take(max + 1)
        .read_to_end(&mut body)
        .map_err(|error| error.to_string())?;
    if body.len() > max as usize {
        return Err(format!(
            "HTTP response exceeds the configured maximum number of bytes: {max}"
        ));
    }
    Ok(body)
}

#[derive(Default)]
struct ChallengeSolverRequestOptions<'a> {
    original_body: Option<Vec<u8>>,
    original_timeout: Option<Duration>,
    max_http_response_bytes: Option<u64>,
    destination_cooldown_key: Option<&'a scryer_outbound_http::DestinationKey>,
}

fn execute_challenge_solver_request(
    proxy_client: &Client,
    request_client: &Client,
    policy: &IndexerProxyPolicy,
    request: &PluginHttpRequest,
    options: ChallengeSolverRequestOptions<'_>,
) -> HostResult<ProxiedHttpResponse> {
    let ChallengeSolverRequestOptions {
        original_body,
        original_timeout,
        max_http_response_bytes,
        destination_cooldown_key,
    } = options;
    if !policy.config.is_enabled {
        return Err("Indexer proxy is disabled for this indexer.".to_string());
    }

    let provider = policy.config.provider_type;
    let provider_name = solver::solver_provider_name(provider);
    let endpoint = solver::solver_solve_endpoint(&policy.config.base_url);
    let solver_timeout = scryer_outbound_http::effective_indexer_proxy_request_timeout(
        policy.config.request_timeout_seconds,
    );
    let solver_deadline = Instant::now() + solver_timeout;
    tracing::debug!(
        indexer_id = policy.indexer_id.as_str(),
        indexer_name = policy.indexer_name.as_str(),
        proxy_config_id = policy.config.id.as_str(),
        proxy_provider = policy.config.provider_type.as_str(),
        request_url = solver::sanitized_url_for_log(&request.url).as_str(),
        "challenge solver request started"
    );

    let response = scryer_outbound_http::send_blocking_reqwest_request_with_cooldown_budget_until(
        proxy_client
            .post(&endpoint)
            .timeout(solver_timeout)
            .json(&solver::solver_solve_request(
                provider,
                &request.url,
                policy.config.request_timeout_seconds,
            )),
        Some(Duration::ZERO),
        solver_deadline,
    )
    .map_err(|error| match error {
        scryer_outbound_http::BlockingOutboundHttpError::Request(error) if error.is_timeout() => {
            solver::solver_error_message(provider, solver::SolverErrorKind::Timeout).to_string()
        }
        scryer_outbound_http::BlockingOutboundHttpError::Request(_) => {
            solver::solver_error_message(provider, solver::SolverErrorKind::Unreachable).to_string()
        }
        scryer_outbound_http::BlockingOutboundHttpError::CooldownBudgetExceeded { .. } => {
            solver::solver_error_message(provider, solver::SolverErrorKind::Unavailable).to_string()
        }
        scryer_outbound_http::BlockingOutboundHttpError::DeadlineExceeded => {
            solver::solver_error_message(provider, solver::SolverErrorKind::Timeout).to_string()
        }
    })?;

    let solver_status = response.status();
    if solver_status == StatusCode::TOO_MANY_REQUESTS || solver_status.is_server_error() {
        tracing::warn!(
            indexer_id = policy.indexer_id.as_str(),
            proxy_config_id = policy.config.id.as_str(),
            status = solver_status.as_u16(),
            "challenge solver service unavailable for indexer proxy request"
        );
        return Err(
            solver::solver_error_message(provider, solver::SolverErrorKind::Unavailable)
                .to_string(),
        );
    }

    let response_body = read_response_body(response, max_http_response_bytes)?;
    let solution = solver::parse_solver_solution(&response_body)
        .map_err(|error| error.message(provider).to_string())?;

    let solution_status = solution.status.unwrap_or_else(|| solver_status.as_u16());
    let solved_final_url = solution.url.as_deref().map(solver::sanitized_url_for_log);
    let solved_body = solution.response.clone().unwrap_or_default().into_bytes();
    let headers = solver::safe_solution_response_headers(solution.headers.as_ref());
    let solved_response = |terminal_error| ProxiedHttpResponse {
        status_code: solution_status,
        captured_response: CapturedIndexerHttpResponse {
            status: solution_status,
            headers: captured_headers_from_text(&headers),
            body: solved_body.clone(),
        },
        headers: headers.clone(),
        body: solved_body.clone(),
        terminal_error,
    };
    if solution_status == StatusCode::TOO_MANY_REQUESTS.as_u16() {
        tracing::warn!(
            indexer_id = policy.indexer_id.as_str(),
            proxy_config_id = policy.config.id.as_str(),
            status = solution_status,
            "challenge solver reported target indexer rate limit"
        );
        return Ok(solved_response(Some(solver::target_rate_limit_message(
            &solution,
        ))));
    }

    if solver::solved_body_looks_rate_limited(&solved_body) {
        return Ok(solved_response(Some(solver::target_rate_limit_message(
            &solution,
        ))));
    }
    if (200..300).contains(&solution_status) && !solved_body.is_empty() {
        // Cache the clearance session so follow-up requests to this origin skip
        // the solver until the session expires or stops clearing challenges.
        solver::SolvedSessionCache::shared().store_solution(
            &policy.config.id,
            &request.url,
            &solution,
        );
        tracing::debug!(
            indexer_id = policy.indexer_id.as_str(),
            proxy_config_id = policy.config.id.as_str(),
            status = solution_status,
            response_bytes = solved_body.len(),
            final_url = solved_final_url.as_deref(),
            "challenge solver response used"
        );
        return Ok(ProxiedHttpResponse {
            status_code: solution_status,
            captured_response: CapturedIndexerHttpResponse {
                status: solution_status,
                headers: captured_headers_from_text(&headers),
                body: solved_body.clone(),
            },
            headers,
            body: solved_body,
            terminal_error: None,
        });
    }

    let retry_headers = solver::solution_retry_headers(&solution);
    if !retry_headers.is_empty() {
        tracing::debug!(
            indexer_id = policy.indexer_id.as_str(),
            proxy_config_id = policy.config.id.as_str(),
            "retrying original request with challenge solver headers"
        );
        let retry = execute_request_with_extra_headers(
            request_client,
            request,
            original_body,
            original_timeout,
            &retry_headers,
            destination_cooldown_key,
        )?;
        let status = retry.status();
        let headers = response_headers(&retry);
        let captured_headers = captured_response_headers(&retry);
        let body = read_response_body(retry, max_http_response_bytes)?;
        let terminal_error = if status == StatusCode::TOO_MANY_REQUESTS {
            let retry_after = solver::header_value(&headers, "retry-after").and_then(|value| {
                scryer_outbound_http::parse_retry_after(value).map(|(delay, _)| delay)
            });
            Some(solver::rate_limit_message_with_retry_after(retry_after))
        } else if !status.is_success() {
            Some(
                solver::solver_error_message(provider, solver::SolverErrorKind::MissingSolution)
                    .to_string(),
            )
        } else {
            None
        };
        if terminal_error.is_none() {
            solver::SolvedSessionCache::shared().store_solution(
                &policy.config.id,
                &request.url,
                &solution,
            );
        }
        return Ok(ProxiedHttpResponse {
            status_code: status.as_u16(),
            captured_response: CapturedIndexerHttpResponse {
                status: status.as_u16(),
                headers: captured_headers,
                body: body.clone(),
            },
            headers,
            body,
            terminal_error,
        });
    }

    if !(200..300).contains(&solution_status) {
        return Ok(solved_response(Some(format!(
            "{provider_name} target request returned HTTP {solution_status}."
        ))));
    }

    Ok(solved_response(Some(
        solver::solver_error_message(provider, solver::SolverErrorKind::MissingSolution)
            .to_string(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingIndexerErrorRecorder {
        errors: Mutex<Vec<scryer_application::NewIndexerError>>,
    }

    impl IndexerErrorRecorder for RecordingIndexerErrorRecorder {
        fn record(
            &self,
            error: scryer_application::NewIndexerError,
        ) -> scryer_application::AppResult<()> {
            self.errors.lock().expect("recorded errors").push(error);
            Ok(())
        }
    }

    /// Stands in for the real store, whose `indexer_errors.indexer_id` foreign
    /// key rejects any id without an `indexers` row.
    #[derive(Default)]
    struct ForeignKeyRejectingRecorder {
        attempts: Mutex<Vec<String>>,
    }

    impl IndexerErrorRecorder for ForeignKeyRejectingRecorder {
        fn record(
            &self,
            error: scryer_application::NewIndexerError,
        ) -> scryer_application::AppResult<()> {
            self.attempts
                .lock()
                .expect("recorded attempts")
                .push(error.indexer_id.clone());
            Err(scryer_application::AppError::Repository(
                "FOREIGN KEY constraint failed".to_string(),
            ))
        }
    }

    fn captured_failure_response() -> CapturedIndexerHttpResponse {
        CapturedIndexerHttpResponse {
            status: 401,
            headers: Vec::new(),
            body: b"unauthorized".to_vec(),
        }
    }

    /// A connection test probes under a synthetic id that has no `indexers`
    /// row. Persisting history for it can only fail the foreign key, so the
    /// capture path must not attempt the write at all — the operator has to see
    /// the probe's own error, not a storage failure raised behind it.
    #[test]
    fn the_connection_test_id_is_never_persisted_as_error_history() {
        let recorder = Arc::new(ForeignKeyRejectingRecorder::default());
        let capture = IndexerErrorCaptureContext {
            indexer_id: scryer_application::CONNECTION_TEST_INDEXER_ID.to_string(),
            indexer_name: "Test Connection".to_string(),
            operation: IndexerErrorOperation::ConnectionTest,
            recorder: recorder.clone(),
        };

        PluginHttpHost::record_captured_response(&capture, captured_failure_response());
        PluginHttpHost::record_transport_failure(&capture);

        assert!(
            recorder
                .attempts
                .lock()
                .expect("recorded attempts")
                .is_empty(),
            "the synthetic connection-test id must never reach the store"
        );
    }

    /// The guard is scoped to the synthetic id: a real indexer still records.
    #[test]
    fn a_stored_indexer_id_still_records_error_history() {
        let recorder = Arc::new(RecordingIndexerErrorRecorder::default());
        let capture = IndexerErrorCaptureContext {
            indexer_id: "indexer-1".to_string(),
            indexer_name: "Test indexer".to_string(),
            operation: IndexerErrorOperation::ConnectionTest,
            recorder: recorder.clone(),
        };

        PluginHttpHost::record_captured_response(&capture, captured_failure_response());

        let errors = recorder.errors.lock().expect("recorded errors");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].indexer_id, "indexer-1");
    }

    const TEST_PLUGIN_HTTP_CA_CERT_PEM: &str = concat!(
        "-----BEGIN CERTIFICATE-----\n",
        "MIIDITCCAgmgAwIBAgIUY40m7DS0vG3xUR0EXxPLYFVq/WkwDQYJKoZIhvcNAQEL\n",
        "BQAwGDEWMBQGA1UEAwwNZTJlLWppbWFrdS1jYTAeFw0yNjA1MjExNzE4NTNaFw0z\n",
        "NjA1MTgxNzE4NTNaMBgxFjAUBgNVBAMMDWUyZS1qaW1ha3UtY2EwggEiMA0GCSqG\n",
        "SIb3DQEBAQUAA4IBDwAwggEKAoIBAQCygxcuiabmKSdpOdnE2Vg9x8AxDtsv3apm\n",
        "qaAeDTaG2uPeSjQsxKJfYDkRmOS9eqEV+yYQeiRwAdq3vadUd/eVlfvvrCtCswkx\n",
        "vHhDvKpgc8KW239IdygK8JFHJz1FTfZRfgWgiKGnlqef6R1w8BjewD6/byv+VJxR\n",
        "cQaVmrBfc7ZzXL41C/WCpdZLMyzRn1EeoEvTYqn1+Yqhhx8WlIQlT2Ha3gOIvAAX\n",
        "Xh1CyfosZbFGfuVk4njM01K00N8GaMk0CWwMvgKADPKNh29S1Pv4PnL5k03Qb4gS\n",
        "bAMRWJi+xMYmtAdINPnJscPKj++vOMdJxGQunpgkXKoHELZWLOANAgMBAAGjYzBh\n",
        "MB8GA1UdIwQYMBaAFMJFcy1sAajZvY0Amv6QuPe4iqPUMA8GA1UdEwEB/wQFMAMB\n",
        "Af8wDgYDVR0PAQH/BAQDAgEGMB0GA1UdDgQWBBTCRXMtbAGo2b2NAJr+kLj3uIqj\n",
        "1DANBgkqhkiG9w0BAQsFAAOCAQEAIZkWiXfdJSLtHUlqUfT5R9ko8acIt1uQt2kI\n",
        "3SiDqyFrHWTT+cyfFyqBIEASPLX9fgPHkz42K4P1Kc9W4JR8o/QWRK7A0hvbCzuB\n",
        "Z/5+agQ15hA1priLKk/oqoILFhT3LHR3/6mzk6vJ3EmIyDITUZ6tQiQS0zyXCxpR\n",
        "8aCN5dsNaBwN42hxBrm/7TjiNCdX54zjLg6cPbtrsHnAI7NBi3O/WNEYISiUcC5O\n",
        "FnEYx13QF8BQo/cY55EZDrEnF4+R6Q3DPQJHhd6tIoEYvxp8wVnUjQb3nWib1wvW\n",
        "dlYNMnHca3kyT/MHY4oX5MmPsHY8ANxBBz0XSKw5ysN4cNpK/Q==\n",
        "-----END CERTIFICATE-----\n",
    );

    #[tokio::test(flavor = "multi_thread")]
    async fn capture_scope_records_failed_http_and_terminal_success_responses_once() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let recorder = Arc::new(RecordingIndexerErrorRecorder::default());
        let host = PluginHttpHost::new(vec!["127.0.0.1".to_string()], None, None, Some(64 * 1024));
        let context = || IndexerErrorCaptureContext {
            indexer_id: "indexer-1".to_string(),
            indexer_name: "Test indexer".to_string(),
            operation: IndexerErrorOperation::InteractiveSearch,
            recorder: recorder.clone(),
        };

        Mock::given(method("GET"))
            .and(path("/unauthorized"))
            .respond_with(
                ResponseTemplate::new(401)
                    .insert_header("content-type", "application/octet-stream")
                    .set_body_bytes(vec![0, 255, 1, 254]),
            )
            .expect(1)
            .mount(&server)
            .await;

        host.begin_indexer_error_capture(context());
        host.request(
            "newznab",
            PluginHttpRequest {
                url: format!("{}/unauthorized", server.uri()),
                method: Some("GET".to_string()),
                headers: BTreeMap::new(),
            },
            None,
            Duration::from_secs(2),
        )
        .expect("accepted HTTP failure response");
        host.finish_indexer_error_capture(true);

        {
            let errors = recorder.errors.lock().expect("recorded errors");
            assert_eq!(
                errors.len(),
                1,
                "terminal completion must not duplicate 401"
            );
            assert_eq!(errors[0].response.as_ref().unwrap().status, 401);
            assert_eq!(
                errors[0].response.as_ref().unwrap().body,
                vec![0, 255, 1, 254]
            );
            assert_eq!(
                errors[0].classification,
                scryer_application::IndexerErrorClassification::HttpUnauthorized
            );
        }

        Mock::given(method("GET"))
            .and(path("/malformed"))
            .respond_with(ResponseTemplate::new(200).set_body_string("malformed plugin result"))
            .expect(1)
            .mount(&server)
            .await;

        host.begin_indexer_error_capture(context());
        host.request(
            "newznab",
            PluginHttpRequest {
                url: format!("{}/malformed", server.uri()),
                method: Some("GET".to_string()),
                headers: BTreeMap::new(),
            },
            None,
            Duration::from_secs(2),
        )
        .expect("accepted HTTP success response");
        host.finish_indexer_error_capture(true);

        let errors = recorder.errors.lock().expect("recorded errors");
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[1].response.as_ref().unwrap().status, 200);
        assert_eq!(
            errors[1].response.as_ref().unwrap().body,
            b"malformed plugin result"
        );
        assert_eq!(
            errors[1].classification,
            scryer_application::IndexerErrorClassification::Unknown
        );
    }

    #[test]
    fn capture_scope_records_failed_operations_without_an_http_response() {
        let recorder = Arc::new(RecordingIndexerErrorRecorder::default());
        let host = PluginHttpHost::new(vec![], None, None, Some(64 * 1024));
        host.begin_indexer_error_capture(IndexerErrorCaptureContext {
            indexer_id: "indexer-1".to_string(),
            indexer_name: "Test indexer".to_string(),
            operation: IndexerErrorOperation::AutomaticSearch,
            recorder: recorder.clone(),
        });

        host.finish_indexer_error_capture(true);

        let errors = recorder.errors.lock().expect("recorded errors");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].response, None);
        assert_eq!(
            errors[0].classification,
            scryer_application::IndexerErrorClassification::Unknown
        );
        assert_eq!(
            errors[0].message,
            "Indexer plugin command failed without an HTTP response"
        );
    }

    #[test]
    fn rate_limit_message_parses_prowlarr_newznab_429_contract() {
        let mut headers = BTreeMap::new();
        headers.insert("retry-after".to_string(), "321".to_string());
        let body = br#"<?xml version="1.0" encoding="UTF-8"?>
<error code="429" description="Indexer is disabled till 8/9/2026 4:30:00 PM due to recent failures." />"#;

        assert_eq!(
            direct_rate_limit_message(&headers, body),
            "HTTP 429: Indexer is disabled till 8/9/2026 4:30:00 PM due to recent failures.; retry after 321s"
        );
    }

    #[test]
    fn rate_limit_message_does_not_guess_non_prowlarr_xml_shapes() {
        let body = br#"<error><description>not Prowlarr's Newznab shape</description></error>"#;

        assert_eq!(
            direct_rate_limit_message(&BTreeMap::new(), body),
            "HTTP 429: rate limited"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn eleven_managed_children_isolate_one_prowlarr_429() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        for child in 0..11 {
            let template = if child == 0 {
                ResponseTemplate::new(429)
                    .insert_header("Content-Type", "application/rss+xml")
                    .insert_header("Retry-After", "60")
                    .set_body_raw(
                        r#"<?xml version="1.0" encoding="UTF-8"?>
<error code="429" description="User configurable Indexer Query Limit of 100 in last 1 hour(s) reached." />"#,
                        "application/rss+xml",
                    )
            } else {
                ResponseTemplate::new(200).set_body_string("ok")
            };
            Mock::given(method("GET"))
                .and(path(format!("/api/{child}")))
                .respond_with(template)
                .mount(&server)
                .await;
        }

        let server_uri = server.uri();
        let errored_children = tokio::task::spawn_blocking(move || {
            let mut errored_children = Vec::new();
            for child in 0..11 {
                let host = PluginHttpHost::new(
                    vec!["127.0.0.1".to_string()],
                    None,
                    Some(format!("managed-indexer:parent:{child}")),
                    Some(64 * 1024),
                );
                let body = host
                    .request(
                        "newznab",
                        PluginHttpRequest {
                            url: format!("{server_uri}/api/{child}"),
                            method: Some("GET".to_string()),
                            headers: BTreeMap::new(),
                        },
                        None,
                        Duration::from_secs(2),
                    )
                    .expect("sibling child request should remain dispatchable");
                let rate_limit_message = host.rate_limit_message("newznab").unwrap();
                if rate_limit_message.is_some() {
                    errored_children.push(child);
                    assert!(String::from_utf8_lossy(&body).contains("Indexer Query Limit"));
                } else {
                    assert_eq!(body, b"ok");
                }
            }
            errored_children
        })
        .await
        .unwrap();

        assert_eq!(errored_children, vec![0]);
        assert_eq!(server.received_requests().await.unwrap().len(), 11);
    }

    #[tokio::test]
    async fn request_reuses_its_worker_from_an_async_runtime() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;

        let host = PluginHttpHost::new(vec!["127.0.0.1".to_string()], None, None, Some(64 * 1024));
        for query in ["first", "second"] {
            let body = host
                .request(
                    "newznab",
                    PluginHttpRequest {
                        url: format!("{}/api?query={query}", server.uri()),
                        method: Some("GET".to_string()),
                        headers: BTreeMap::new(),
                    },
                    None,
                    Duration::from_secs(2),
                )
                .expect("plugin HTTP request should succeed from an async runtime");

            assert_eq!(body, b"ok");
        }
    }

    #[test]
    fn request_client_cache_key_reuses_an_origin_across_search_queries() {
        let first = plugin_http_request_client_key("https://indexer.example/api?t=search&q=first")
            .expect("first search URL should be valid");
        let second =
            plugin_http_request_client_key("https://indexer.example/api?t=search&q=second")
                .expect("second search URL should be valid");
        let different_port = plugin_http_request_client_key("https://indexer.example:8443/api")
            .expect("alternate origin URL should be valid");

        assert_eq!(first, second);
        assert_ne!(first, different_port);
    }

    #[test]
    fn pinned_client_cache_entries_expire_after_five_minutes() {
        assert!(pinned_client_is_fresh(Instant::now()));
        assert!(!pinned_client_is_fresh(
            Instant::now() - PINNED_REQUEST_CLIENT_TTL
        ));
    }

    #[test]
    fn worker_response_timeout_includes_dispatch_grace() {
        assert_eq!(
            worker_response_timeout(Duration::from_secs(5)),
            Duration::from_secs(5) + PLUGIN_HTTP_WORKER_RESPONSE_GRACE
        );
    }

    #[test]
    fn build_plugin_http_client_accepts_empty_trust_bundle() {
        scryer_outbound_http::blocking_plugin_host_client("")
            .expect("default trust bundle should build");
    }

    #[test]
    fn build_plugin_http_client_accepts_uploaded_certificates() {
        scryer_outbound_http::blocking_plugin_host_client(TEST_PLUGIN_HTTP_CA_CERT_PEM)
            .expect("uploaded trust bundle should build");
    }

    #[test]
    fn add_uploaded_certificates_rejects_non_certificate_pem_items() {
        let error = scryer_outbound_http::blocking_plugin_host_client(
            "-----BEGIN PRIVATE KEY-----\nZmFrZQ==\n-----END PRIVATE KEY-----\n",
        )
        .expect_err("non-certificate bundle should be rejected");

        assert!(error.contains("uploaded trusted certificate bundle"));
    }

    #[test]
    fn solved_cookies_overlay_matching_names_and_preserve_original_cookies() {
        let merged = merge_cookie_headers(
            Some("auth=secret; cf_clearance=stale; theme=dark"),
            "cf_clearance=fresh; bot_session=ready",
        );

        assert_eq!(
            merged.as_deref(),
            Some("auth=secret; cf_clearance=fresh; theme=dark; bot_session=ready")
        );
    }

    #[test]
    fn enforce_allowed_hosts_rejects_disallowed_hosts() {
        let error = enforce_allowed_hosts(
            Some(&["example.com".to_string()]),
            "https://jimaku.example.test/search",
        )
        .expect_err("disallowed host should fail");

        assert!(error.to_string().contains("is not allowed"));
    }

    #[test]
    fn enforce_allowed_hosts_error_omits_query_credentials() {
        let error = enforce_allowed_hosts(
            Some(&["allowed.example".to_string()]),
            "https://tracker.example.test/download?apikey=SECRETKEY&passkey=TOPSECRET",
        )
        .expect_err("disallowed host should fail");

        assert!(error.contains("is not allowed"));
        assert!(
            !error.contains('?'),
            "error must not carry a query string: {error}"
        );
        assert!(
            !error.contains("apikey"),
            "error must not leak apikey: {error}"
        );
        assert!(
            !error.contains("passkey"),
            "error must not leak passkey: {error}"
        );
        assert!(
            !error.contains("SECRETKEY"),
            "error must not leak secrets: {error}"
        );
        assert!(
            !error.contains("TOPSECRET"),
            "error must not leak secrets: {error}"
        );
    }

    #[test]
    fn plugin_request_egress_blocks_cloud_metadata() {
        let result = scryer_outbound_http::prepare_plugin_blocking_http_target(
            "http://169.254.169.254/latest/meta-data/",
            "",
            "plugin HTTP",
        );

        assert!(
            matches!(
                result,
                Err(
                    scryer_outbound_http::OutboundDestinationError::BlockedLinkLocalOrMetadata { .. }
                )
            ),
            "cloud metadata address must be rejected on the plugin HTTP host path"
        );
    }

    #[test]
    fn plugin_request_egress_allows_loopback_companion() {
        scryer_outbound_http::prepare_plugin_blocking_http_target(
            "http://127.0.0.1:9117/api",
            "",
            "plugin HTTP",
        )
        .expect("loopback companion must be allowed for self-hosted plugins");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn trawl_solver_response_is_used_for_plugin_indexer_request() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let target_url = "https://indexer.example/api?t=search";
        Mock::given(method("POST"))
            .and(path("/v1"))
            .and(body_json(serde_json::json!({
                "cmd": "request.get",
                "url": target_url,
                "maxTimeout": 60_000
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ok",
                "solution": {
                    "url": target_url,
                    "status": 200,
                    "headers": {},
                    "cookies": [{ "name": "cf_clearance", "value": "abc" }],
                    "userAgent": "Trawl UA",
                    "response": "<html>Trawl</html>"
                }
            })))
            .mount(&server)
            .await;

        let now = chrono::Utc::now();
        let policy = IndexerProxyPolicy {
            indexer_id: "indexer-1".into(),
            indexer_name: "Indexer".into(),
            config: scryer_domain::IndexerProxyConfig {
                id: "trawl-1".into(),
                name: "Trawl".into(),
                provider_type: scryer_domain::IndexerProxyProviderType::Trawl,
                protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
                base_url: server.uri(),
                request_timeout_seconds: 60,
                is_enabled: true,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                created_at: now,
                updated_at: now,
            },
        };
        let request = PluginHttpRequest {
            url: target_url.into(),
            method: Some("GET".into()),
            headers: BTreeMap::new(),
        };

        let solved = tokio::task::spawn_blocking(move || {
            let proxy_client = scryer_outbound_http::blocking_indexer_proxy_reqwest_client("")
                .expect("proxy client");
            let request_client =
                scryer_outbound_http::blocking_plugin_host_client("").expect("request client");
            execute_challenge_solver_request(
                &proxy_client,
                &request_client,
                &policy,
                &request,
                ChallengeSolverRequestOptions {
                    max_http_response_bytes: Some(1024 * 1024),
                    ..Default::default()
                },
            )
        })
        .await
        .expect("solver task should join")
        .expect("Trawl plugin solve should succeed");

        assert_eq!(solved.status_code, 200);
        assert_eq!(solved.body, b"<html>Trawl</html>");
        assert!(solved.headers.is_empty());
        let requests = server.received_requests().await.expect("recorded requests");
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]
                .headers
                .get("user-agent")
                .and_then(|value| value.to_str().ok()),
            Some(scryer_outbound_http::INDEXER_PROXY_USER_AGENT)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn byparr_non_success_solution_refetches_with_clearance_session() {
        use wiremock::matchers::{body_json, header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let target_url = format!("{}/api", server.uri());
        Mock::given(method("POST"))
            .and(path("/v1"))
            .and(body_json(serde_json::json!({
                "cmd": "request.get",
                "url": target_url,
                "maxTimeout": 60
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ok",
                "solution": {
                    "url": target_url,
                    "status": 503,
                    "headers": { "content-type": "text/html" },
                    "cookies": [{ "name": "cf_clearance", "value": "abc" }],
                    "userAgent": "Byparr UA",
                    "response": "<html>Just a moment</html>"
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api"))
            .and(header("cookie", "cf_clearance=abc"))
            .and(header("user-agent", "Byparr UA"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/xml")
                    .set_body_bytes(b"<rss></rss>"),
            )
            .mount(&server)
            .await;

        let now = chrono::Utc::now();
        let policy = IndexerProxyPolicy {
            indexer_id: "indexer-1".into(),
            indexer_name: "Indexer".into(),
            config: scryer_domain::IndexerProxyConfig {
                id: "byparr-1".into(),
                name: "Byparr".into(),
                provider_type: scryer_domain::IndexerProxyProviderType::Byparr,
                protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
                base_url: server.uri(),
                request_timeout_seconds: 60,
                is_enabled: true,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                created_at: now,
                updated_at: now,
            },
        };
        let request = PluginHttpRequest {
            url: target_url,
            method: Some("GET".into()),
            headers: BTreeMap::new(),
        };

        let solved = tokio::task::spawn_blocking(move || {
            let proxy_client = scryer_outbound_http::blocking_indexer_proxy_reqwest_client("")
                .expect("proxy client");
            let request_client =
                scryer_outbound_http::blocking_plugin_host_client("").expect("request client");
            execute_challenge_solver_request(
                &proxy_client,
                &request_client,
                &policy,
                &request,
                ChallengeSolverRequestOptions {
                    max_http_response_bytes: Some(1024 * 1024),
                    ..Default::default()
                },
            )
        })
        .await
        .expect("solver task should join")
        .expect("Byparr session refetch should succeed");

        assert_eq!(solved.status_code, 200);
        assert_eq!(solved.body, b"<rss></rss>");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn trawl_server_error_is_classified_as_solver_unavailable() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let target_url = "https://indexer.example/api?t=search";
        Mock::given(method("POST"))
            .and(path("/v1"))
            .and(body_json(serde_json::json!({
                "cmd": "request.get",
                "url": target_url,
                "maxTimeout": 60_000
            })))
            .respond_with(ResponseTemplate::new(503).set_body_json(serde_json::json!({
                "status": "error",
                "message": "Browser pool initializing, retry in a few seconds",
                "solution": {
                    "url": target_url,
                    "status": 0,
                    "headers": {},
                    "response": "",
                    "cookies": [],
                    "userAgent": ""
                }
            })))
            .mount(&server)
            .await;

        let now = chrono::Utc::now();
        let policy = IndexerProxyPolicy {
            indexer_id: "indexer-1".into(),
            indexer_name: "Indexer".into(),
            config: scryer_domain::IndexerProxyConfig {
                id: "trawl-unavailable".into(),
                name: "Trawl".into(),
                provider_type: scryer_domain::IndexerProxyProviderType::Trawl,
                protocol: scryer_domain::ChallengeSolverProtocol::RequestSolutionV1,
                base_url: server.uri(),
                request_timeout_seconds: 60,
                is_enabled: true,
                last_health_status: None,
                last_error_message: None,
                last_error_at: None,
                created_at: now,
                updated_at: now,
            },
        };
        let request = PluginHttpRequest {
            url: target_url.into(),
            method: Some("GET".into()),
            headers: BTreeMap::new(),
        };

        let result = tokio::task::spawn_blocking(move || {
            let proxy_client = scryer_outbound_http::blocking_indexer_proxy_reqwest_client("")
                .expect("proxy client");
            let request_client =
                scryer_outbound_http::blocking_plugin_host_client("").expect("request client");
            execute_challenge_solver_request(
                &proxy_client,
                &request_client,
                &policy,
                &request,
                ChallengeSolverRequestOptions {
                    max_http_response_bytes: Some(1024 * 1024),
                    ..Default::default()
                },
            )
        })
        .await
        .expect("solver task should join");
        let error = match result {
            Ok(_) => panic!("Trawl server errors must fail as solver unavailable"),
            Err(error) => error,
        };

        assert_eq!(error, solver::TRAWL_UNAVAILABLE_MESSAGE);
    }
}
