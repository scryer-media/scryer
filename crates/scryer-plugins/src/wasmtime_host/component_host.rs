//! Async WASI Preview 2 host for indexer components.
//!
//! The component ABI intentionally has a single HTTP operation.  Indexer
//! plugins own fanout, pacing, quotas, and retries; this module owns only the
//! per-attempt security boundary and resource limits.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use scryer_application::{
    CapturedIndexerHttpHeader, CapturedIndexerHttpResponse, challenge_solver as solver,
};
use tokio::sync::mpsc;
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::plugin_http_host::{
    IndexerErrorCaptureContext, IndexerProxyPolicy, PluginHttpHost, enforce_allowed_hosts,
    shared_plugin_http_runtime,
};
use crate::wasmtime_host::sandbox::HostLimits;

mod contract_v1_0 {
    wasmtime::component::bindgen!({
        world: "scryer:indexer/indexer-plugin@1.0.0",
        path: "wit",
    });
}

mod contract_v1_1 {
    wasmtime::component::bindgen!({
        world: "scryer:indexer/indexer-plugin@1.1.0",
        path: "wit/indexer-v1.1.0",
    });
}

use self::contract_v1_0::scryer::indexer::host::{
    Header, Host, HostWithStore, HttpRequest, HttpResponse, LogLevel, TransportError,
};

const MAX_COMPONENT_HTTP_PER_ACTOR: usize = 256;
const MAX_COMPONENT_HTTP_PROCESS: usize = 1024;
const MAX_COMPONENT_STATE_BYTES: usize = 1024 * 1024;
const DEFAULT_MAX_HTTP_RESPONSE_BYTES: usize = 50 * 1024 * 1024;
pub(crate) const COMPONENT_STRATEGY_EVENT_CAPACITY: usize = 16;

static ACTIVE_COMPONENT_HTTP: AtomicUsize = AtomicUsize::new(0);

/// Returns true only for a top-level component binary, whatever world it
/// implements. Every component-backed plugin kind classifies on this first and
/// lets its generated binding validate the concrete package/world during
/// instantiation.
pub(crate) fn is_component_binary(wasm: &[u8]) -> Result<bool, String> {
    let mut parser = wasmparser::Parser::new(0);
    match parser.parse(wasm, true) {
        Ok(wasmparser::Chunk::Parsed {
            payload: wasmparser::Payload::Version { encoding, .. },
            ..
        }) => Ok(matches!(encoding, wasmparser::Encoding::Component)),
        Ok(_) => Ok(false),
        Err(error) => Err(format!("invalid WASM binary: {error}")),
    }
}

pub(crate) fn validate_indexer_component(wasm: &[u8]) -> Result<(), String> {
    ComponentRuntime::new(crate::wasmtime_host::engine::shared_async_engine(), wasm).map(|_| ())
}

pub(crate) fn component_strategy_event_channel() -> (mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>)
{
    mpsc::channel(COMPONENT_STRATEGY_EVENT_CAPACITY)
}

#[derive(Clone)]
pub(crate) struct ComponentHost {
    inner: Arc<ComponentHostInner>,
}

struct ComponentHostInner {
    config: BTreeMap<String, String>,
    provider_profile: Option<Vec<u8>>,
    allowed_hosts: Vec<String>,
    indexer_proxy_policy: Option<IndexerProxyPolicy>,
    timeout: Duration,
    max_response_bytes: usize,
    clock_origin: std::time::Instant,
    operation_deadline: Mutex<std::time::Instant>,
    state: Mutex<ComponentState>,
    active: AtomicUsize,
    cancellation: Mutex<tokio_util::sync::CancellationToken>,
    indexer_error_capture: Mutex<Option<ComponentIndexerErrorCapture>>,
    strategy_events: Mutex<StrategyEventState>,
}

#[derive(Default)]
struct ComponentState {
    values: BTreeMap<String, Vec<u8>>,
    bytes: usize,
}

struct ComponentIndexerErrorCapture {
    context: IndexerErrorCaptureContext,
    final_response: Option<CapturedIndexerHttpResponse>,
}

#[derive(Default)]
struct StrategyEventState {
    next_scope_id: u64,
    active: Option<ActiveStrategyEventSink>,
}

struct ActiveStrategyEventSink {
    scope_id: u64,
    sender: mpsc::Sender<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StrategyEventSinkError {
    NoActivePlan,
    Closed,
}

struct StrategyPlanScope {
    host: ComponentHost,
    scope_id: u64,
}

impl Drop for StrategyPlanScope {
    fn drop(&mut self) {
        let Ok(mut state) = self.host.inner.strategy_events.lock() else {
            return;
        };
        if state
            .active
            .as_ref()
            .is_some_and(|active| active.scope_id == self.scope_id)
        {
            state.active = None;
        }
    }
}

impl ComponentHost {
    #[cfg(test)]
    pub(crate) fn for_indexer(
        config: BTreeMap<String, String>,
        allowed_hosts: Vec<String>,
        indexer_proxy_policy: Option<IndexerProxyPolicy>,
        timeout: Duration,
        max_http_response_bytes: Option<u64>,
    ) -> Result<Self, String> {
        Self::for_indexer_with_provider_profile(
            config,
            allowed_hosts,
            indexer_proxy_policy,
            timeout,
            max_http_response_bytes,
            None,
        )
    }

    pub(crate) fn for_indexer_with_provider_profile(
        config: BTreeMap<String, String>,
        allowed_hosts: Vec<String>,
        indexer_proxy_policy: Option<IndexerProxyPolicy>,
        timeout: Duration,
        max_http_response_bytes: Option<u64>,
        provider_profile: Option<Vec<u8>>,
    ) -> Result<Self, String> {
        Ok(Self {
            inner: Arc::new(ComponentHostInner {
                config,
                provider_profile,
                allowed_hosts,
                indexer_proxy_policy,
                timeout,
                max_response_bytes: max_http_response_bytes
                    .and_then(|value| usize::try_from(value).ok())
                    .unwrap_or(DEFAULT_MAX_HTTP_RESPONSE_BYTES),
                clock_origin: std::time::Instant::now(),
                operation_deadline: Mutex::new(std::time::Instant::now() + timeout),
                state: Mutex::new(ComponentState::default()),
                active: AtomicUsize::new(0),
                cancellation: Mutex::new(tokio_util::sync::CancellationToken::new()),
                indexer_error_capture: Mutex::new(None),
                strategy_events: Mutex::new(StrategyEventState::default()),
            }),
        })
    }

    /// Calls against one component actor are serialized by its adapter, so
    /// replacing this token before each operation cannot race another guest.
    /// The token lives outside the actor Store so an actor recreation preserves
    /// configuration and state but not the cancelled invocation.
    pub(crate) fn bind_cancellation(&self, token: tokio_util::sync::CancellationToken) {
        if let Ok(mut cancellation) = self.inner.cancellation.lock() {
            *cancellation = token;
        }
    }

    fn cancellation(&self) -> tokio_util::sync::CancellationToken {
        self.inner
            .cancellation
            .lock()
            .map(|token| token.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }

    fn remaining_operation_timeout(&self) -> Result<Duration, TransportError> {
        let deadline = self
            .inner
            .operation_deadline
            .lock()
            .map(|deadline| *deadline)
            .unwrap_or_else(|poisoned| *poisoned.into_inner());
        deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or(TransportError::Timeout)
    }

    fn provider_profile(&self) -> Option<Vec<u8>> {
        self.inner.provider_profile.clone()
    }

    fn begin_strategy_plan(
        &self,
        sender: mpsc::Sender<Vec<u8>>,
    ) -> Result<StrategyPlanScope, String> {
        if sender.is_closed() {
            return Err("indexer component strategy event sink is closed".to_string());
        }
        let mut state = self
            .inner
            .strategy_events
            .lock()
            .map_err(|_| "indexer component strategy event state is unavailable".to_string())?;
        if state.active.is_some() {
            return Err("indexer component strategy plan is already active".to_string());
        }
        state.next_scope_id = state.next_scope_id.wrapping_add(1);
        let scope_id = state.next_scope_id;
        state.active = Some(ActiveStrategyEventSink { scope_id, sender });
        Ok(StrategyPlanScope {
            host: self.clone(),
            scope_id,
        })
    }

    async fn emit_strategy_event(&self, event: Vec<u8>) -> Result<(), StrategyEventSinkError> {
        let sender = {
            let state = self
                .inner
                .strategy_events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state
                .active
                .as_ref()
                .map(|active| active.sender.clone())
                .ok_or(StrategyEventSinkError::NoActivePlan)?
        };
        if sender.is_closed() {
            return Err(StrategyEventSinkError::Closed);
        }
        let cancellation = self.cancellation();
        tokio::select! {
            _ = cancellation.cancelled() => Err(StrategyEventSinkError::Closed),
            result = sender.send(event) => result.map_err(|_| StrategyEventSinkError::Closed),
        }
    }

    pub(crate) fn new_store(&self, engine: &Engine) -> Store<ComponentCtx> {
        let mut store = Store::new(
            engine,
            ComponentCtx {
                table: ResourceTable::new(),
                wasi: WasiCtxBuilder::new().build(),
                host: self.clone(),
                limits: HostLimits::new(None),
            },
        );
        store.limiter(|ctx: &mut ComponentCtx| &mut ctx.limits);
        self.begin_operation(&mut store);
        store
    }

    /// Starts a fresh operation budget immediately before entering the guest.
    /// `Store::set_epoch_deadline` is relative to the engine's current epoch,
    /// so it must be renewed for every retained actor invocation.
    pub(crate) fn begin_operation(&self, store: &mut Store<ComponentCtx>) {
        let deadline = std::time::Instant::now() + self.inner.timeout;
        if let Ok(mut operation_deadline) = self.inner.operation_deadline.lock() {
            *operation_deadline = deadline;
        }
        store.set_epoch_deadline(crate::wasmtime_host::engine::deadline_ticks(
            self.inner.timeout,
        ));
    }

    async fn http(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        if self.cancellation().is_cancelled() {
            return Err(TransportError::Cancelled);
        }
        let per_actor = self.inner.active.fetch_add(1, Ordering::AcqRel) + 1;
        if per_actor > MAX_COMPONENT_HTTP_PER_ACTOR {
            self.inner.active.fetch_sub(1, Ordering::AcqRel);
            return Err(TransportError::Capacity);
        }
        let process = ACTIVE_COMPONENT_HTTP.fetch_add(1, Ordering::AcqRel) + 1;
        if process > MAX_COMPONENT_HTTP_PROCESS {
            ACTIVE_COMPONENT_HTTP.fetch_sub(1, Ordering::AcqRel);
            self.inner.active.fetch_sub(1, Ordering::AcqRel);
            return Err(TransportError::Capacity);
        }
        let _guard = ActiveRequestGuard {
            actor: &self.inner.active,
        };

        enforce_allowed_hosts(Some(&self.inner.allowed_hosts), &request.url)
            .map_err(|_| TransportError::ForbiddenOrigin)?;
        let extra_ca_bundle_pem = shared_plugin_http_runtime()
            .extra_ca_bundle_pem()
            .map_err(|_| TransportError::Transport)?;
        let cancellation = self.cancellation();
        let target = tokio::select! {
            _ = cancellation.cancelled() => return Err(TransportError::Cancelled),
            result = scryer_outbound_http::prepare_plugin_http_target_with_extra_ca(
                &request.url,
                &extra_ca_bundle_pem,
                "component plugin HTTP",
            ) => result.map_err(component_outbound_destination_error)?,
        };
        let Some(policy) = self.inner.indexer_proxy_policy.as_ref() else {
            let direct = self
                .send_direct_request(target.client(), target.url(), &request, &[])
                .await?;
            self.capture_indexer_response(direct.captured_response.clone());
            return Ok(direct.response);
        };

        // A configured solver owns the initial target attempt. A guest GET may
        // first validate a cached clearance session; if that response is still
        // challenged, the stale session is invalidated and the same request
        // falls through to a fresh solve. A successful solve can require one
        // replay against the already-prepared origin so the guest receives raw
        // origin bytes rather than browser-rendered solver content. These
        // attempts remain one logical guest operation under the same deadline.
        let session_headers = request.method.eq_ignore_ascii_case("GET").then(|| {
            solver::SolvedSessionCache::shared().session_headers(&policy.config.id, &request.url)
        });
        if let Some(session_headers) = session_headers.filter(|headers| !headers.is_empty()) {
            let direct = self
                .send_direct_request(target.client(), target.url(), &request, &session_headers)
                .await?;
            if solver::looks_like_challenge_response(
                direct.response.status,
                &component_header_map(&direct.response.headers),
                &direct.response.body,
            ) {
                solver::SolvedSessionCache::shared().invalidate(&policy.config.id, &request.url);
            } else {
                self.capture_indexer_response(direct.captured_response.clone());
                return Ok(direct.response);
            }
        }
        if !request.method.eq_ignore_ascii_case("GET") {
            return Err(TransportError::Transport);
        }
        let solved = self
            .solve_proxy_request(
                target.client(),
                target.url(),
                &request,
                policy,
                &extra_ca_bundle_pem,
            )
            .await?;
        self.capture_indexer_response(solved.captured_response.clone());
        Ok(solved.response)
    }

    async fn send_direct_request(
        &self,
        client: &reqwest::Client,
        url: &reqwest::Url,
        request: &HttpRequest,
        extra_headers: &[(String, String)],
    ) -> Result<ComponentHttpResult, TransportError> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| TransportError::InvalidRequest)?;
        let timeout = self.remaining_operation_timeout()?;
        let mut builder = client.request(method, url.clone()).timeout(timeout);
        let merged_cookie = component_merged_cookie(request, extra_headers);
        for header in &request.headers {
            if extra_headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case(&header.name))
            {
                continue;
            }
            builder = builder.header(&header.name, &header.value);
        }
        for (name, value) in extra_headers {
            if name.eq_ignore_ascii_case("cookie") {
                if let Some(cookie) = merged_cookie.as_deref() {
                    builder = builder.header(name, cookie);
                }
            } else {
                builder = builder.header(name, value);
            }
        }
        if !request.body.is_empty() {
            builder = builder.body(request.body.clone());
        }
        let cancellation = self.cancellation();
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(TransportError::Cancelled),
            result = builder.send() => result.map_err(component_transport_error)?,
        };
        self.read_response(response).await
    }

    async fn solve_proxy_request(
        &self,
        target_client: &reqwest::Client,
        target_url: &reqwest::Url,
        request: &HttpRequest,
        policy: &IndexerProxyPolicy,
        extra_ca_bundle_pem: &str,
    ) -> Result<ComponentHttpResult, TransportError> {
        if !policy.config.is_enabled {
            return Err(TransportError::Transport);
        }
        let provider = policy.config.provider_type;
        let solver_timeout = scryer_outbound_http::effective_indexer_proxy_request_timeout(
            policy.config.request_timeout_seconds,
        )
        .min(self.remaining_operation_timeout()?);
        let proxy_client =
            scryer_outbound_http::indexer_proxy_reqwest_client_with_extra_ca(extra_ca_bundle_pem)
                .map_err(|_| {
                solver::SolverHealthLedger::shared().record_failure(
                    &policy.config.id,
                    solver::solver_error_message(provider, solver::SolverErrorKind::Unreachable),
                );
                TransportError::Transport
            })?;
        let endpoint = solver::solver_solve_endpoint(&policy.config.base_url);
        let cancellation = self.cancellation();
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(TransportError::Cancelled),
            result = proxy_client
                .post(&endpoint)
                .timeout(solver_timeout)
                .json(&solver::solver_solve_request(
                    provider,
                    &request.url,
                    policy.config.request_timeout_seconds,
                ))
                .send() => match result {
                    Ok(response) => response,
                    Err(error) => {
                        let kind = if error.is_timeout() {
                            solver::SolverErrorKind::Timeout
                        } else {
                            solver::SolverErrorKind::Unreachable
                        };
                        solver::SolverHealthLedger::shared().record_failure(
                            &policy.config.id,
                            solver::solver_error_message(provider, kind),
                        );
                        return Err(component_transport_error(error));
                    }
                },
        };
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
            || response.status().is_server_error()
        {
            solver::SolverHealthLedger::shared().record_failure(
                &policy.config.id,
                solver::solver_error_message(provider, solver::SolverErrorKind::Unavailable),
            );
            return Err(TransportError::Transport);
        }
        let (_, _, _, solver_body) = self.read_raw_response(response).await?;
        let solution = match solver::parse_solver_solution(&solver_body) {
            Ok(solution) => solution,
            Err(error) => {
                let message = error.message(provider);
                if solver::is_solver_service_error_message(message) {
                    solver::SolverHealthLedger::shared().record_failure(&policy.config.id, message);
                }
                return Err(TransportError::Transport);
            }
        };
        solver::SolverHealthLedger::shared().record_success(&policy.config.id);

        let status = solution.status.unwrap_or(200);
        let headers = solver::safe_solution_response_headers(solution.headers.as_ref());
        let body = solution.response.clone().unwrap_or_default().into_bytes();
        if body.len() > self.inner.max_response_bytes {
            return Err(TransportError::ResponseTooLarge);
        }
        let retry_headers = solver::solution_retry_headers(&solution);
        if !retry_headers.is_empty() {
            let replay = self
                .send_direct_request(target_client, target_url, request, &retry_headers)
                .await?;
            if !solver::looks_like_challenge_response(
                replay.response.status,
                &component_header_map(&replay.response.headers),
                &replay.response.body,
            ) {
                solver::SolvedSessionCache::shared().store_solution(
                    &policy.config.id,
                    &request.url,
                    &solution,
                );
            }
            return Ok(replay);
        }
        Ok(component_solved_response(status, headers, body))
    }

    async fn read_response(
        &self,
        response: reqwest::Response,
    ) -> Result<ComponentHttpResult, TransportError> {
        let (status, headers, captured_headers, body) = self.read_raw_response(response).await?;
        Ok(ComponentHttpResult {
            captured_response: CapturedIndexerHttpResponse {
                status,
                headers: captured_headers,
                body: body.clone(),
            },
            response: HttpResponse {
                status,
                headers,
                body,
            },
        })
    }

    async fn read_raw_response(
        &self,
        mut response: reqwest::Response,
    ) -> Result<(u16, Vec<Header>, Vec<CapturedIndexerHttpHeader>, Vec<u8>), TransportError> {
        if response
            .content_length()
            .is_some_and(|length| length > self.inner.max_response_bytes as u64)
        {
            return Err(TransportError::ResponseTooLarge);
        }
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value.to_str().ok().map(|value| Header {
                    name: name.as_str().to_string(),
                    value: value.to_string(),
                })
            })
            .collect();
        let captured_headers = response
            .headers()
            .iter()
            .map(|(name, value)| CapturedIndexerHttpHeader {
                name: name.as_str().to_string(),
                value: value.as_bytes().to_vec(),
            })
            .collect();
        let mut body = Vec::new();
        loop {
            let cancellation = self.cancellation();
            let chunk = tokio::select! {
                _ = cancellation.cancelled() => return Err(TransportError::Cancelled),
                result = response.chunk() => result.map_err(component_transport_error)?,
            };
            let Some(chunk) = chunk else {
                break;
            };
            if body
                .len()
                .checked_add(chunk.len())
                .is_none_or(|length| length > self.inner.max_response_bytes)
            {
                return Err(TransportError::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok((status, headers, captured_headers, body))
    }

    pub(crate) fn begin_indexer_error_capture(&self, context: IndexerErrorCaptureContext) {
        match self.inner.indexer_error_capture.lock() {
            Ok(mut capture) => {
                *capture = Some(ComponentIndexerErrorCapture {
                    context,
                    final_response: None,
                });
            }
            Err(error) => {
                tracing::warn!(error = %error, "failed to start component indexer HTTP error capture")
            }
        }
    }

    pub(crate) fn finish_indexer_error_capture(&self, operation_failed: bool) {
        let capture = match self.inner.indexer_error_capture.lock() {
            Ok(mut capture) => capture.take(),
            Err(error) => {
                tracing::warn!(error = %error, "failed to finish component indexer HTTP error capture");
                None
            }
        };
        let Some(capture) = capture else {
            return;
        };
        let Some(response) = capture.final_response else {
            if operation_failed {
                PluginHttpHost::record_transport_failure(&capture.context);
            }
            return;
        };
        if operation_failed && (200..300).contains(&response.status) {
            PluginHttpHost::record_captured_response(&capture.context, response);
        }
    }

    fn capture_indexer_response(&self, response: CapturedIndexerHttpResponse) {
        let immediate_capture = match self.inner.indexer_error_capture.lock() {
            Ok(mut capture) => {
                let Some(capture) = capture.as_mut() else {
                    return;
                };
                capture.final_response = Some(response.clone());
                (!(200..300).contains(&response.status)).then(|| capture.context.clone())
            }
            Err(error) => {
                tracing::warn!(error = %error, "failed to capture component indexer HTTP response");
                None
            }
        };
        if let Some(capture) = immediate_capture {
            PluginHttpHost::record_captured_response(&capture, response);
        }
    }

    fn state_get(&self, key: &str) -> Option<Vec<u8>> {
        self.inner
            .state
            .lock()
            .ok()
            .and_then(|state| state.values.get(key).cloned())
    }

    fn state_cas(
        &self,
        key: String,
        expected: Option<Vec<u8>>,
        replacement: Option<Vec<u8>>,
    ) -> bool {
        let Ok(mut state) = self.inner.state.lock() else {
            return false;
        };
        if state.values.get(&key).cloned() != expected {
            return false;
        }
        let old = state
            .values
            .get(&key)
            .map(|value| key.len() + value.len())
            .unwrap_or(0);
        let new = replacement
            .as_ref()
            .map(|value| key.len() + value.len())
            .unwrap_or(0);
        let Some(total) = state
            .bytes
            .checked_sub(old)
            .and_then(|bytes| bytes.checked_add(new))
        else {
            return false;
        };
        if total > MAX_COMPONENT_STATE_BYTES {
            return false;
        }
        match replacement {
            Some(value) => {
                state.values.insert(key, value);
            }
            None => {
                state.values.remove(&key);
            }
        }
        state.bytes = total;
        true
    }
}

struct ComponentHttpResult {
    response: HttpResponse,
    captured_response: CapturedIndexerHttpResponse,
}

fn component_transport_error(error: reqwest::Error) -> TransportError {
    if error.is_timeout() {
        TransportError::Timeout
    } else if error.is_builder() {
        TransportError::InvalidRequest
    } else {
        TransportError::Transport
    }
}

fn component_outbound_destination_error(
    error: scryer_outbound_http::OutboundDestinationError,
) -> TransportError {
    use scryer_outbound_http::OutboundDestinationError;

    match error {
        OutboundDestinationError::InvalidUrl { .. }
        | OutboundDestinationError::UnsupportedScheme { .. }
        | OutboundDestinationError::MissingHost { .. } => TransportError::InvalidRequest,
        OutboundDestinationError::EmbeddedCredentials { .. }
        | OutboundDestinationError::ForbiddenAddress { .. }
        | OutboundDestinationError::BlockedLinkLocalOrMetadata { .. } => {
            TransportError::ForbiddenOrigin
        }
        OutboundDestinationError::ResolveFailed { .. }
        | OutboundDestinationError::NoResolvedAddresses { .. }
        | OutboundDestinationError::ClientBuild { .. }
        | OutboundDestinationError::TrustBundle { .. } => TransportError::Transport,
    }
}

fn component_header_map(headers: &[Header]) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|header| (header.name.clone(), header.value.clone()))
        .collect()
}

fn component_merged_cookie(
    request: &HttpRequest,
    extra_headers: &[(String, String)],
) -> Option<String> {
    let solved_cookie = extra_headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("cookie"))
        .map(|(_, value)| value.as_str());
    let original_cookie = request
        .headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case("cookie"))
        .map(|header| header.value.as_str());
    let solved_cookie = solved_cookie?;

    let mut cookies: Vec<(String, String)> = Vec::new();
    for header in original_cookie
        .into_iter()
        .chain(std::iter::once(solved_cookie))
    {
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
            if let Some((_, existing)) = cookies.iter_mut().find(|(existing, _)| existing == &name)
            {
                *existing = value;
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

fn component_solved_response(
    status: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
) -> ComponentHttpResult {
    let headers: Vec<_> = headers
        .into_iter()
        .map(|(name, value)| Header { name, value })
        .collect();
    let captured_headers = headers
        .iter()
        .map(|header| CapturedIndexerHttpHeader {
            name: header.name.clone(),
            value: header.value.as_bytes().to_vec(),
        })
        .collect();
    ComponentHttpResult {
        captured_response: CapturedIndexerHttpResponse {
            status,
            headers: captured_headers,
            body: body.clone(),
        },
        response: HttpResponse {
            status,
            headers,
            body,
        },
    }
}

struct ActiveRequestGuard<'a> {
    actor: &'a AtomicUsize,
}

impl Drop for ActiveRequestGuard<'_> {
    fn drop(&mut self) {
        self.actor.fetch_sub(1, Ordering::AcqRel);
        ACTIVE_COMPONENT_HTTP.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) struct ComponentCtx {
    table: ResourceTable,
    wasi: WasiCtx,
    host: ComponentHost,
    limits: HostLimits,
}

impl WasiView for ComponentCtx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl Host for ComponentCtx {
    fn monotonic_now_ms(&mut self) -> u64 {
        self.host
            .inner
            .clock_origin
            .elapsed()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }

    fn wall_now_ms(&mut self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }

    fn operation_deadline_monotonic_ms(&mut self) -> u64 {
        let deadline = self
            .host
            .inner
            .operation_deadline
            .lock()
            .map(|deadline| *deadline)
            .unwrap_or_else(|poisoned| *poisoned.into_inner());
        deadline
            .saturating_duration_since(self.host.inner.clock_origin)
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }

    fn config_get(&mut self, key: String) -> Option<String> {
        self.host.inner.config.get(&key).cloned()
    }

    fn state_get(&mut self, key: String) -> Option<Vec<u8>> {
        self.host.state_get(&key)
    }

    fn state_cas(
        &mut self,
        key: String,
        expected: Option<Vec<u8>>,
        replacement: Option<Vec<u8>>,
    ) -> bool {
        self.host.state_cas(key, expected, replacement)
    }

    fn log(&mut self, level: LogLevel, message: String) {
        match level {
            LogLevel::Trace => tracing::trace!(target: "scryer_plugins::component", "{message}"),
            LogLevel::Debug => tracing::debug!(target: "scryer_plugins::component", "{message}"),
            LogLevel::Info => tracing::info!(target: "scryer_plugins::component", "{message}"),
            LogLevel::Warn => tracing::warn!(target: "scryer_plugins::component", "{message}"),
            LogLevel::Error => tracing::error!(target: "scryer_plugins::component", "{message}"),
        }
    }
}

impl contract_v1_1::scryer::indexer::host::Host for ComponentCtx {
    fn monotonic_now_ms(&mut self) -> u64 {
        self.host
            .inner
            .clock_origin
            .elapsed()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }

    fn wall_now_ms(&mut self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }

    fn operation_deadline_monotonic_ms(&mut self) -> u64 {
        let deadline = self
            .host
            .inner
            .operation_deadline
            .lock()
            .map(|deadline| *deadline)
            .unwrap_or_else(|poisoned| *poisoned.into_inner());
        deadline
            .saturating_duration_since(self.host.inner.clock_origin)
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }

    fn config_get(&mut self, key: String) -> Option<String> {
        self.host.inner.config.get(&key).cloned()
    }

    fn provider_profile(&mut self) -> Option<Vec<u8>> {
        self.host.provider_profile()
    }

    fn state_get(&mut self, key: String) -> Option<Vec<u8>> {
        self.host.state_get(&key)
    }

    fn state_cas(
        &mut self,
        key: String,
        expected: Option<Vec<u8>>,
        replacement: Option<Vec<u8>>,
    ) -> bool {
        self.host.state_cas(key, expected, replacement)
    }

    fn log(&mut self, level: contract_v1_1::scryer::indexer::host::LogLevel, message: String) {
        use contract_v1_1::scryer::indexer::host::LogLevel as V1LogLevel;

        match level {
            V1LogLevel::Trace => tracing::trace!(target: "scryer_plugins::component", "{message}"),
            V1LogLevel::Debug => tracing::debug!(target: "scryer_plugins::component", "{message}"),
            V1LogLevel::Info => tracing::info!(target: "scryer_plugins::component", "{message}"),
            V1LogLevel::Warn => tracing::warn!(target: "scryer_plugins::component", "{message}"),
            V1LogLevel::Error => tracing::error!(target: "scryer_plugins::component", "{message}"),
        }
    }
}

impl HostWithStore<ComponentCtx> for HasSelf<ComponentCtx> {
    fn http(
        accessor: &wasmtime::component::Accessor<ComponentCtx, Self>,
        request: HttpRequest,
    ) -> impl std::future::Future<Output = Result<HttpResponse, TransportError>> + Send {
        let host = accessor.with(|mut access| access.get().host.clone());
        async move { host.http(request).await }
    }

    fn sleep(
        accessor: &wasmtime::component::Accessor<ComponentCtx, Self>,
        duration_ms: u64,
    ) -> impl std::future::Future<Output = ()> + Send {
        let cancellation = accessor.with(|mut access| access.get().host.cancellation());
        async move {
            tokio::select! {
                _ = cancellation.cancelled() => (),
                _ = tokio::time::sleep(Duration::from_millis(duration_ms)) => (),
            }
        }
    }
}

impl contract_v1_1::scryer::indexer::host::HostWithStore<ComponentCtx> for HasSelf<ComponentCtx> {
    fn http(
        accessor: &wasmtime::component::Accessor<ComponentCtx, Self>,
        request: contract_v1_1::scryer::indexer::host::HttpRequest,
    ) -> impl std::future::Future<
        Output = Result<
            contract_v1_1::scryer::indexer::host::HttpResponse,
            contract_v1_1::scryer::indexer::host::TransportError,
        >,
    > + Send {
        let host = accessor.with(|mut access| access.get().host.clone());
        async move {
            let request = HttpRequest {
                method: request.method,
                url: request.url,
                headers: request
                    .headers
                    .into_iter()
                    .map(|header| Header {
                        name: header.name,
                        value: header.value,
                    })
                    .collect(),
                body: request.body,
            };
            host.http(request)
                .await
                .map(
                    |response| contract_v1_1::scryer::indexer::host::HttpResponse {
                        status: response.status,
                        headers: response
                            .headers
                            .into_iter()
                            .map(|header| contract_v1_1::scryer::indexer::host::Header {
                                name: header.name,
                                value: header.value,
                            })
                            .collect(),
                        body: response.body,
                    },
                )
                .map_err(v1_1_transport_error)
        }
    }

    fn sleep(
        accessor: &wasmtime::component::Accessor<ComponentCtx, Self>,
        duration_ms: u64,
    ) -> impl std::future::Future<Output = ()> + Send {
        let cancellation = accessor.with(|mut access| access.get().host.cancellation());
        async move {
            tokio::select! {
                _ = cancellation.cancelled() => (),
                _ = tokio::time::sleep(Duration::from_millis(duration_ms)) => (),
            }
        }
    }

    fn emit_strategy_event(
        accessor: &wasmtime::component::Accessor<ComponentCtx, Self>,
        event: Vec<u8>,
    ) -> impl std::future::Future<
        Output = Result<(), contract_v1_1::scryer::indexer::host::StrategyEventError>,
    > + Send {
        let host = accessor.with(|mut access| access.get().host.clone());
        async move {
            host.emit_strategy_event(event)
                .await
                .map_err(|error| match error {
                    StrategyEventSinkError::NoActivePlan => {
                        contract_v1_1::scryer::indexer::host::StrategyEventError::NoActivePlan
                    }
                    StrategyEventSinkError::Closed => {
                        contract_v1_1::scryer::indexer::host::StrategyEventError::Closed
                    }
                })
        }
    }
}

fn v1_1_transport_error(
    error: TransportError,
) -> contract_v1_1::scryer::indexer::host::TransportError {
    use contract_v1_1::scryer::indexer::host::TransportError as V1TransportError;

    match error {
        TransportError::InvalidRequest => V1TransportError::InvalidRequest,
        TransportError::ForbiddenOrigin => V1TransportError::ForbiddenOrigin,
        TransportError::Timeout => V1TransportError::Timeout,
        TransportError::Cancelled => V1TransportError::Cancelled,
        TransportError::ResponseTooLarge => V1TransportError::ResponseTooLarge,
        TransportError::Capacity => V1TransportError::Capacity,
        TransportError::Transport => V1TransportError::Transport,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ComponentContractVersion {
    V1_0,
    V1_1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ComponentInvocationError {
    Failed,
    Cancelled,
    InvalidResponse,
}

enum ComponentInstancePre {
    V1_0(contract_v1_0::IndexerPluginPre<ComponentCtx>),
    V1_1(contract_v1_1::IndexerPluginPre<ComponentCtx>),
}

pub(crate) struct ComponentRuntime {
    pub(crate) component: Arc<Component>,
    instance_pre: ComponentInstancePre,
}

impl ComponentRuntime {
    pub(crate) fn new(engine: &Engine, wasm: &[u8]) -> Result<Self, String> {
        let component = crate::wasmtime_host::module_cache::indexer_component(wasm)?;
        if !Engine::same(component.engine(), engine) {
            return Err(
                "indexer component cache returned an artifact for a different engine".into(),
            );
        }
        let mut linker = Linker::new(engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|error| format!("failed to register WASI Preview 2: {error:#}"))?;
        contract_v1_0::IndexerPlugin::add_to_linker::<ComponentCtx, HasSelf<ComponentCtx>>(
            &mut linker,
            |ctx| ctx,
        )
        .map_err(|error| format!("failed to register indexer 1.0 component host: {error:#}"))?;
        contract_v1_1::IndexerPlugin::add_to_linker::<ComponentCtx, HasSelf<ComponentCtx>>(
            &mut linker,
            |ctx| ctx,
        )
        .map_err(|error| format!("failed to register indexer 1.1 component host: {error:#}"))?;
        let raw_instance_pre = linker
            .instantiate_pre(&component)
            .map_err(|error| format!("failed to preinstantiate indexer component: {error:#}"))?;
        let instance_pre = match contract_v1_1::IndexerPluginPre::new(raw_instance_pre) {
            Ok(instance_pre) => ComponentInstancePre::V1_1(instance_pre),
            Err(v1_1_error) => {
                let raw_instance_pre = linker.instantiate_pre(&component).map_err(|error| {
                    format!("failed to preinstantiate indexer component: {error:#}")
                })?;
                contract_v1_0::IndexerPluginPre::new(raw_instance_pre)
                    .map(ComponentInstancePre::V1_0)
                    .map_err(|v1_0_error| {
                        format!(
                            "indexer component exports are incompatible with 1.1 ({v1_1_error:#}) and 1.0 ({v1_0_error:#})"
                        )
                    })?
            }
        };
        Ok(Self {
            component,
            instance_pre,
        })
    }

    pub(crate) const fn contract_version(&self) -> ComponentContractVersion {
        match &self.instance_pre {
            ComponentInstancePre::V1_0(_) => ComponentContractVersion::V1_0,
            ComponentInstancePre::V1_1(_) => ComponentContractVersion::V1_1,
        }
    }

    pub(crate) async fn instantiate(&self, host: &ComponentHost) -> Result<ComponentActor, String> {
        let mut store = host.new_store(self.component.engine());
        let plugin = match &self.instance_pre {
            ComponentInstancePre::V1_0(instance_pre) => instance_pre
                .instantiate_async(&mut store)
                .await
                .map(ComponentPlugin::V1_0),
            ComponentInstancePre::V1_1(instance_pre) => instance_pre
                .instantiate_async(&mut store)
                .await
                .map(ComponentPlugin::V1_1),
        }
        .map_err(|error| format!("failed to instantiate indexer component: {error:#}"))?;
        Ok(ComponentActor { store, plugin })
    }
}

enum ComponentPlugin {
    V1_0(contract_v1_0::IndexerPlugin),
    V1_1(contract_v1_1::IndexerPlugin),
}

/// A component instance is retained per configured indexer. The adapter owns
/// its serialization and deliberately drops this whole value on cancellation,
/// timeout, or trap; that drops outstanding component HTTP futures too.
pub(crate) struct ComponentActor {
    store: Store<ComponentCtx>,
    plugin: ComponentPlugin,
}

impl ComponentActor {
    pub(crate) async fn search(
        &mut self,
        request: Vec<u8>,
    ) -> Result<Result<Vec<u8>, ComponentInvocationError>, String> {
        let host = self.store.data().host.clone();
        host.begin_operation(&mut self.store);
        match &self.plugin {
            ComponentPlugin::V1_0(plugin) => self
                .store
                .run_concurrent(async move |accessor| plugin.call_search(accessor, request).await)
                .await
                .map_err(|error| format!("indexer component search scheduling failed: {error:#}"))?
                .map_err(|error| format!("indexer component search failed: {error:#}"))
                .map(|result| result.map_err(component_invocation_error_v1_0)),
            ComponentPlugin::V1_1(plugin) => self
                .store
                .run_concurrent(async move |accessor| plugin.call_search(accessor, request).await)
                .await
                .map_err(|error| format!("indexer component search scheduling failed: {error:#}"))?
                .map_err(|error| format!("indexer component search failed: {error:#}"))
                .map(|result| result.map_err(component_invocation_error_v1_1)),
        }
    }

    pub(crate) async fn search_plan(
        &mut self,
        request: Vec<u8>,
        event_sink: mpsc::Sender<Vec<u8>>,
    ) -> Result<Result<Vec<u8>, ComponentInvocationError>, String> {
        let ComponentPlugin::V1_1(plugin) = &self.plugin else {
            return Err(
                "indexer component contract 1.0 does not support strategy plans".to_string(),
            );
        };
        let host = self.store.data().host.clone();
        let _scope = host.begin_strategy_plan(event_sink)?;
        host.begin_operation(&mut self.store);
        self.store
            .run_concurrent(async move |accessor| plugin.call_search_plan(accessor, request).await)
            .await
            .map_err(|error| {
                format!("indexer component strategy plan scheduling failed: {error:#}")
            })?
            .map_err(|error| format!("indexer component strategy plan failed: {error:#}"))
            .map(|result| result.map_err(component_invocation_error_v1_1))
    }

    pub(crate) async fn action(
        &mut self,
        request: Vec<u8>,
    ) -> Result<Result<Vec<u8>, ComponentInvocationError>, String> {
        let host = self.store.data().host.clone();
        host.begin_operation(&mut self.store);
        match &self.plugin {
            ComponentPlugin::V1_0(plugin) => self
                .store
                .run_concurrent(async move |accessor| plugin.call_action(accessor, request).await)
                .await
                .map_err(|error| format!("indexer component action scheduling failed: {error:#}"))?
                .map_err(|error| format!("indexer component action failed: {error:#}"))
                .map(|result| result.map_err(component_invocation_error_v1_0)),
            ComponentPlugin::V1_1(plugin) => self
                .store
                .run_concurrent(async move |accessor| plugin.call_action(accessor, request).await)
                .await
                .map_err(|error| format!("indexer component action scheduling failed: {error:#}"))?
                .map_err(|error| format!("indexer component action failed: {error:#}"))
                .map(|result| result.map_err(component_invocation_error_v1_1)),
        }
    }
}

fn component_invocation_error_v1_0(
    error: contract_v1_0::InvocationError,
) -> ComponentInvocationError {
    match error {
        contract_v1_0::InvocationError::Failed => ComponentInvocationError::Failed,
        contract_v1_0::InvocationError::Cancelled => ComponentInvocationError::Cancelled,
        contract_v1_0::InvocationError::InvalidResponse => {
            ComponentInvocationError::InvalidResponse
        }
    }
}

fn component_invocation_error_v1_1(
    error: contract_v1_1::InvocationError,
) -> ComponentInvocationError {
    match error {
        contract_v1_1::InvocationError::Failed => ComponentInvocationError::Failed,
        contract_v1_1::InvocationError::Cancelled => ComponentInvocationError::Cancelled,
        contract_v1_1::InvocationError::InvalidResponse => {
            ComponentInvocationError::InvalidResponse
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strategy_test_host(provider_profile: Option<Vec<u8>>) -> ComponentHost {
        ComponentHost::for_indexer_with_provider_profile(
            BTreeMap::new(),
            Vec::new(),
            None,
            Duration::from_secs(1),
            None,
            provider_profile,
        )
        .expect("component host should build")
    }

    #[tokio::test]
    async fn strategy_events_require_an_active_plan() {
        let host = strategy_test_host(None);

        assert_eq!(
            host.emit_strategy_event(vec![1]).await,
            Err(StrategyEventSinkError::NoActivePlan)
        );
    }

    #[tokio::test]
    async fn strategy_plan_scope_streams_and_then_closes() {
        let host = strategy_test_host(Some(vec![9, 8, 7]));
        let (sender, mut receiver) = component_strategy_event_channel();
        let scope = host
            .begin_strategy_plan(sender)
            .expect("strategy plan should begin");

        host.emit_strategy_event(vec![1, 2, 3])
            .await
            .expect("strategy event should be accepted");
        assert_eq!(receiver.recv().await, Some(vec![1, 2, 3]));
        assert_eq!(host.provider_profile(), Some(vec![9, 8, 7]));

        drop(scope);
        assert_eq!(
            host.emit_strategy_event(vec![4]).await,
            Err(StrategyEventSinkError::NoActivePlan)
        );
    }

    #[tokio::test]
    async fn strategy_event_sink_reports_a_closed_receiver() {
        let host = strategy_test_host(None);
        let (sender, receiver) = component_strategy_event_channel();
        let _scope = host
            .begin_strategy_plan(sender)
            .expect("strategy plan should begin");
        drop(receiver);

        assert_eq!(
            host.emit_strategy_event(vec![1]).await,
            Err(StrategyEventSinkError::Closed)
        );
    }

    #[tokio::test]
    async fn strategy_event_sink_is_bounded() {
        let host = strategy_test_host(None);
        let (sender, _receiver) = component_strategy_event_channel();
        let _scope = host
            .begin_strategy_plan(sender)
            .expect("strategy plan should begin");
        for value in 0..COMPONENT_STRATEGY_EVENT_CAPACITY {
            host.emit_strategy_event(vec![u8::try_from(value).expect("test value fits")])
                .await
                .expect("buffered event should be accepted");
        }

        assert!(
            tokio::time::timeout(
                Duration::from_millis(20),
                host.emit_strategy_event(vec![u8::MAX])
            )
            .await
            .is_err(),
            "an event beyond the fixed channel capacity must apply backpressure"
        );
    }

    #[test]
    fn component_outbound_destination_errors_keep_their_category() {
        use scryer_outbound_http::OutboundDestinationError;

        assert_eq!(
            component_outbound_destination_error(OutboundDestinationError::InvalidUrl {
                label: "test",
                message: "invalid".to_string(),
            }),
            TransportError::InvalidRequest
        );
        assert_eq!(
            component_outbound_destination_error(OutboundDestinationError::UnsupportedScheme {
                label: "test",
            }),
            TransportError::InvalidRequest
        );
        assert_eq!(
            component_outbound_destination_error(OutboundDestinationError::MissingHost {
                label: "test",
            }),
            TransportError::InvalidRequest
        );
        assert_eq!(
            component_outbound_destination_error(OutboundDestinationError::EmbeddedCredentials {
                label: "test",
            }),
            TransportError::ForbiddenOrigin
        );
        assert_eq!(
            component_outbound_destination_error(OutboundDestinationError::ForbiddenAddress {
                label: "test",
                host: "127.0.0.1".to_string(),
            }),
            TransportError::ForbiddenOrigin
        );
        assert_eq!(
            component_outbound_destination_error(
                OutboundDestinationError::BlockedLinkLocalOrMetadata {
                    label: "test",
                    host: "169.254.169.254".to_string(),
                },
            ),
            TransportError::ForbiddenOrigin
        );
        assert_eq!(
            component_outbound_destination_error(OutboundDestinationError::ResolveFailed {
                label: "test",
                host: "unresolvable.invalid".to_string(),
                source: std::io::Error::other("unavailable"),
            }),
            TransportError::Transport
        );
        assert_eq!(
            component_outbound_destination_error(OutboundDestinationError::NoResolvedAddresses {
                label: "test",
                host: "empty.example".to_string(),
            }),
            TransportError::Transport
        );
        assert_eq!(
            component_outbound_destination_error(OutboundDestinationError::TrustBundle {
                label: "test",
                message: "unavailable".to_string(),
            }),
            TransportError::Transport
        );
    }

    #[test]
    fn component_validation_rejects_missing_world_exports() {
        let wasm = wat::parse_str("(component)").expect("component WAT must parse");

        assert!(
            validate_indexer_component(&wasm).is_err(),
            "an arbitrary component must not pass indexer-world validation"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn component_store_enforces_legacy_memory_limit() {
        let host = ComponentHost::for_indexer(
            BTreeMap::new(),
            Vec::new(),
            None,
            Duration::from_secs(1),
            None,
        )
        .expect("component host should build");
        let engine = crate::wasmtime_host::engine::shared_async_engine();
        let module = wasmtime::Module::new(
            engine,
            wat::parse_str("(module (memory (export \"memory\") 0))")
                .expect("core module WAT must parse"),
        )
        .expect("core module must compile");
        let mut store = host.new_store(engine);
        let instance = wasmtime::Instance::new_async(&mut store, &module, &[])
            .await
            .expect("core module must instantiate");
        let memory = instance
            .get_memory(&mut store, "memory")
            .expect("memory export must exist");
        let past_default_limit_pages =
            u64::try_from(crate::wasmtime_host::sandbox::DEFAULT_ARCHIVE_MEMORY_CAP_BYTES / 65536)
                .expect("default memory cap fits in u64")
                + 1;

        assert!(
            memory
                .grow_async(&mut store, past_default_limit_pages)
                .await
                .is_err(),
            "component Store must reject core-memory growth past the legacy cap"
        );
        assert!(
            store.data().limits.memory_denied,
            "the shared limiter must record the rejected growth"
        );
    }

    fn component_solver_test_host(server_uri: String, proxy_id: &str) -> ComponentHost {
        component_solver_test_host_with_timeout(server_uri, proxy_id, Duration::from_secs(120))
    }

    fn component_solver_test_host_with_timeout(
        server_uri: String,
        proxy_id: &str,
        timeout: Duration,
    ) -> ComponentHost {
        let now = chrono::Utc::now();
        ComponentHost::for_indexer(
            BTreeMap::new(),
            vec!["127.0.0.1".to_string()],
            Some(IndexerProxyPolicy {
                indexer_id: "component-indexer".to_string(),
                indexer_name: "Component indexer".to_string(),
                config: scryer_domain::IndexerProxyConfig {
                    id: proxy_id.to_string(),
                    name: "Component solver".to_string(),
                    provider_type: scryer_domain::IndexerProxyProviderType::Trawl,
                    protocol: Some(scryer_domain::ChallengeSolverProtocol::RequestSolutionV1),
                    username_encrypted: None,
                    password_encrypted: None,
                    remote_dns: false,
                    base_url: server_uri,
                    request_timeout_seconds: 60,
                    is_enabled: true,
                    last_health_status: None,
                    last_error_message: None,
                    last_error_at: None,
                    created_at: now,
                    updated_at: now,
                },
            }),
            timeout,
            Some(1024 * 1024),
        )
        .expect("component host should accept a configured solver")
    }

    fn component_solver_request(target_url: String, headers: Vec<Header>) -> HttpRequest {
        HttpRequest {
            method: "GET".to_string(),
            url: target_url,
            headers,
            body: Vec::new(),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn component_http_replays_solver_clearance_and_caches_session() {
        use wiremock::matchers::{body_json, header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let target_url = format!("{}/search", server.uri());
        Mock::given(method("POST"))
            .and(path("/v1"))
            .and(body_json(serde_json::json!({
                "cmd": "request.get",
                "url": target_url,
                "maxTimeout": 60_000,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ok",
                "solution": {
                    "url": "https://solver.example/diagnostic-only",
                    "status": 200,
                    "headers": {"content-type": "application/json"},
                    "cookies": [{"name": "clearance", "value": "solved"}],
                    "userAgent": "solver-test",
                    "response": "<html><body><pre>{\"wrapped\":true}</pre></body></html>",
                },
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(header("user-agent", "solver-test"))
            .and(header("cookie", "original=one; clearance=solved"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_string(r#"{"origin":true}"#),
            )
            .expect(2)
            .mount(&server)
            .await;

        let host = component_solver_test_host(server.uri(), "component-solver-replay-cache");
        let headers = vec![Header {
            name: "cookie".to_string(),
            value: "original=one".to_string(),
        }];
        let first = host
            .http(component_solver_request(
                target_url.clone(),
                headers.clone(),
            ))
            .await
            .expect("solver clearance should replay the origin");
        let second = host
            .http(component_solver_request(target_url, headers))
            .await
            .expect("cached solver clearance should replay the origin");

        assert_eq!(first.status, 200);
        assert_eq!(first.body, br#"{"origin":true}"#);
        assert_eq!(second.status, 200);
        assert_eq!(second.body, br#"{"origin":true}"#);
        assert_eq!(
            server
                .received_requests()
                .await
                .expect("recorded requests")
                .len(),
            3,
            "the second request must use the cached clearance without another solver call"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn component_http_does_not_cache_an_unchallenged_user_agent_only_replay() {
        use wiremock::matchers::{body_json, header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let connection_url = format!("{}/connection", server.uri());
        let search_url = format!("{}/search", server.uri());
        Mock::given(method("POST"))
            .and(path("/v1"))
            .and(body_json(serde_json::json!({
                "cmd": "request.get",
                "url": connection_url,
                "maxTimeout": 60_000,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ok",
                "solution": {
                    "url": connection_url,
                    "status": 200,
                    "headers": {},
                    "cookies": [],
                    "userAgent": "connection-agent",
                    "response": "connection validation",
                },
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/connection"))
            .and(header("user-agent", "connection-agent"))
            .respond_with(ResponseTemplate::new(200).set_body_string("connection origin"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(header("user-agent", "connection-agent"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("content-type", "text/html")
                    .set_body_string("<html><title>Just a moment</title><div>cf-chl</div></html>"),
            )
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1"))
            .and(body_json(serde_json::json!({
                "cmd": "request.get",
                "url": search_url,
                "maxTimeout": 60_000,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ok",
                "solution": {
                    "url": search_url,
                    "status": 200,
                    "headers": {},
                    "cookies": [{"name": "clearance", "value": "fresh"}],
                    "userAgent": "search-agent",
                    "response": "search validation",
                },
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(header("user-agent", "search-agent"))
            .and(header("cookie", "clearance=fresh"))
            .respond_with(ResponseTemplate::new(200).set_body_string("search origin"))
            .expect(1)
            .mount(&server)
            .await;

        let host = component_solver_test_host(server.uri(), "component-solver-cache-admission");
        let connection = host
            .http(component_solver_request(connection_url, Vec::new()))
            .await
            .expect("connection validation should replay its origin response");
        let search = host
            .http(component_solver_request(search_url, Vec::new()))
            .await
            .expect("the challenged search should invoke the solver");

        assert_eq!(connection.body, b"connection origin");
        assert_eq!(search.body, b"search origin");
        assert_eq!(
            server
                .received_requests()
                .await
                .expect("recorded requests")
                .len(),
            4,
            "a user-agent-only connection response must not seed the origin cache"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn component_http_recovers_a_challenged_cached_session_in_the_same_request() {
        use wiremock::matchers::{body_json, header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let target_url = format!("{}/search", server.uri());
        let proxy_id = "component-solver-stale-cache-recovery";
        let stale_solution = solver::ChallengeSolverSolution {
            url: Some(target_url.clone()),
            status: Some(200),
            cookies: Some(vec![serde_json::json!({
                "name": "clearance",
                "value": "stale",
            })]),
            user_agent: Some("stale-agent".to_string()),
            headers: None,
            response: None,
        };
        solver::SolvedSessionCache::shared().store_solution(proxy_id, &target_url, &stale_solution);

        Mock::given(method("GET"))
            .and(path("/search"))
            .and(header("user-agent", "stale-agent"))
            .and(header("cookie", "clearance=stale"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("content-type", "text/html")
                    .set_body_string("<html><title>Just a moment</title><div>cf-chl</div></html>"),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1"))
            .and(body_json(serde_json::json!({
                "cmd": "request.get",
                "url": target_url,
                "maxTimeout": 60_000,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ok",
                "solution": {
                    "url": target_url,
                    "status": 200,
                    "headers": {},
                    "cookies": [{"name": "clearance", "value": "fresh"}],
                    "userAgent": "fresh-agent",
                    "response": "fresh solver response",
                },
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(header("user-agent", "fresh-agent"))
            .and(header("cookie", "clearance=fresh"))
            .respond_with(ResponseTemplate::new(200).set_body_string("fresh origin response"))
            .expect(2)
            .mount(&server)
            .await;

        let host = component_solver_test_host(server.uri(), proxy_id);
        let recovered = host
            .http(component_solver_request(target_url.clone(), Vec::new()))
            .await
            .expect("a stale cache entry should recover within the same request");
        let cached = host
            .http(component_solver_request(target_url, Vec::new()))
            .await
            .expect("the refreshed clearance should be cached");

        assert_eq!(recovered.status, 200);
        assert_eq!(recovered.body, b"fresh origin response");
        assert_eq!(cached.status, 200);
        assert_eq!(cached.body, b"fresh origin response");
        assert_eq!(
            server
                .received_requests()
                .await
                .expect("recorded requests")
                .len(),
            4,
            "recovery should perform one stale attempt, one solve, one replay, and one cached request"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn component_http_shares_one_deadline_across_solver_and_clearance_replay() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let target_url = format!("{}/search", server.uri());
        Mock::given(method("POST"))
            .and(path("/v1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(100))
                    .set_body_json(serde_json::json!({
                        "status": "ok",
                        "solution": {
                            "url": target_url,
                            "status": 200,
                            "headers": {},
                            "cookies": [{"name": "clearance", "value": "solved"}],
                            "userAgent": "solver-test",
                            "response": "<html><body>solver response</body></html>",
                        },
                    })),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(100))
                    .set_body_string("origin response"),
            )
            .mount(&server)
            .await;

        let host = component_solver_test_host_with_timeout(
            server.uri(),
            "component-solver-shared-deadline",
            Duration::from_millis(150),
        );
        let error = host
            .http(component_solver_request(target_url.clone(), Vec::new()))
            .await
            .expect_err("the replay must use only the solver operation's remaining budget");

        assert_eq!(error, TransportError::Timeout);

        let solver_request = server
            .received_requests()
            .await
            .expect("recorded requests")
            .into_iter()
            .find(|request| request.url.path() == "/v1")
            .expect("the solver command should be sent to /v1");
        assert_eq!(solver_request.method.as_str(), "POST");
        let solver_command: serde_json::Value = serde_json::from_slice(&solver_request.body)
            .expect("the solver command should contain JSON");
        assert_eq!(
            solver_command,
            serde_json::json!({
                "cmd": "request.get",
                "url": target_url,
                "maxTimeout": 60_000,
            })
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn component_http_keeps_terminal_solver_response_without_retry_headers() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let target_url = format!("{}/search", server.uri());
        Mock::given(method("POST"))
            .and(path("/v1"))
            .and(body_json(serde_json::json!({
                "cmd": "request.get",
                "url": target_url,
                "maxTimeout": 60_000,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ok",
                "solution": {
                    "url": target_url,
                    "status": 200,
                    "headers": {"content-type": "application/rss+xml"},
                    "cookies": [],
                    "response": "<rss>terminal-solver-response</rss>",
                },
            })))
            .expect(1)
            .mount(&server)
            .await;

        let host = component_solver_test_host(server.uri(), "component-solver-terminal-response");
        let response = host
            .http(component_solver_request(target_url, Vec::new()))
            .await
            .expect("terminal solver response should remain supported");

        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"<rss>terminal-solver-response</rss>");
        assert_eq!(
            server
                .received_requests()
                .await
                .expect("recorded requests")
                .len(),
            1,
            "a terminal solver response without replay headers must not hit the origin"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn component_http_does_not_cache_a_replayed_challenge() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let target_url = format!("{}/search", server.uri());
        Mock::given(method("POST"))
            .and(path("/v1"))
            .and(body_json(serde_json::json!({
                "cmd": "request.get",
                "url": target_url,
                "maxTimeout": 60_000,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ok",
                "solution": {
                    "url": target_url,
                    "status": 200,
                    "headers": {},
                    "cookies": [{"name": "clearance", "value": "stale"}],
                    "userAgent": "solver-test",
                    "response": "<html><body>solver response</body></html>",
                },
            })))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("content-type", "text/html")
                    .set_body_string("<html><title>Just a moment</title><div>cf-chl</div></html>"),
            )
            .expect(2)
            .mount(&server)
            .await;

        let host = component_solver_test_host(server.uri(), "component-solver-stale-clearance");
        let first = host
            .http(component_solver_request(target_url.clone(), Vec::new()))
            .await
            .expect("stale clearance response should be returned to the guest");
        let second = host
            .http(component_solver_request(target_url, Vec::new()))
            .await
            .expect("stale clearance must invoke the solver again");

        assert_eq!(first.status, 403);
        assert_eq!(second.status, 403);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn component_http_preserves_replayed_origin_rate_limits() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let target_url = format!("{}/search", server.uri());
        Mock::given(method("POST"))
            .and(path("/v1"))
            .and(body_json(serde_json::json!({
                "cmd": "request.get",
                "url": target_url,
                "maxTimeout": 60_000,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ok",
                "solution": {
                    "url": target_url,
                    "status": 200,
                    "headers": {},
                    "cookies": [{"name": "clearance", "value": "solved"}],
                    "userAgent": "solver-test",
                    "response": "<html><body><pre>rate limited</pre></body></html>",
                },
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "120")
                    .set_body_string("Request limit reached"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let host = component_solver_test_host(server.uri(), "component-solver-rate-limit");
        let response = host
            .http(component_solver_request(target_url, Vec::new()))
            .await
            .expect("origin rate limit should be returned to the guest");

        assert_eq!(response.status, 429);
        assert_eq!(response.body, b"Request limit reached");
        assert!(response.headers.iter().any(|header| {
            header.name.eq_ignore_ascii_case("retry-after") && header.value == "120"
        }));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn component_http_preserves_replayed_origin_failures() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let target_url = format!("{}/search", server.uri());
        Mock::given(method("POST"))
            .and(path("/v1"))
            .and(body_json(serde_json::json!({
                "cmd": "request.get",
                "url": target_url,
                "maxTimeout": 60_000,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ok",
                "solution": {
                    "url": target_url,
                    "status": 200,
                    "headers": {},
                    "cookies": [{"name": "clearance", "value": "solved"}],
                    "userAgent": "solver-test",
                    "response": "<html><body><pre>upstream failure</pre></body></html>",
                },
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(502).set_body_string("origin unavailable"))
            .expect(1)
            .mount(&server)
            .await;

        let host = component_solver_test_host(server.uri(), "component-solver-origin-failure");
        let response = host
            .http(component_solver_request(target_url, Vec::new()))
            .await
            .expect("origin failure should be returned to the guest");

        assert_eq!(response.status, 502);
        assert_eq!(response.body, b"origin unavailable");
    }
}
