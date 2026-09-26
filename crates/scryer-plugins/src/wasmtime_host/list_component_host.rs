//! WASI Preview 2 host for list-provider components.
//!
//! The subtitle host's 1.1 contract is this file's template: a list provider
//! imports the shared encoded door (`scryer:host/services@1.0.0`) and the
//! typed, family-neutral runtime (`scryer:runtime/host@1.0.0`), and exports a
//! synchronous `describe` plus an `async` `process`. There is one contract
//! revision, so there is no version negotiation here.
//!
//! Nothing in this file implements a service. `host-call` and the typed `http`
//! both go through [`family_component_host::dispatch_host_call`] into the
//! plugin's own [`CommandHost`], so allowed-host enforcement, the response
//! cap, the proxy policy and the per-invocation deadline apply once.
//!
//! Instance-per-request: one `process` call per invocation, then the whole
//! `Store` is dropped, which is also what keeps a member credential carried in
//! a request from outliving the call that needed it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use scryer_application::{AppError, AppResult};
use scryer_plugin_sdk::command::{PluginCommandRequest, PluginCommandResponse};
use scryer_plugin_sdk::host::{
    PluginHostRequest, PluginHostResponse, PluginHttpRequest as SdkPluginHttpRequest,
};
use scryer_plugin_sdk::{PluginDescriptor, PluginError, PluginErrorCode, PluginResult};
use tracing::Instrument;
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::runtime_backing::PluginInstanceSpec;
use crate::wasmtime_host::command_host::CommandHost;
use crate::wasmtime_host::family_component_host::{
    DESCRIBE_TIMEOUT, HostCallError, HostCallFailure, stderr_suffix, tail_of,
};
use crate::wasmtime_host::sandbox::{self, HostLimits, PreparedComponentSandbox};
use crate::wasmtime_host::{engine, error, family_component_host, module_cache};

mod contract {
    wasmtime::component::bindgen!({
        world: "scryer:lists/list-provider@1.0.0",
        // Same layout rule as every family world: one canonical copy of each
        // shared package, pushed before the family package that imports it.
        path: ["wit/host-v1.0.0", "wit/runtime-v1.0.0", "wit/list-v1.0.0"],
        imports: { default: async },
        exports: { default: async },
    });
}

use self::contract::InvocationError;
use self::contract::scryer::host::services::{Host as ServicesHost, HostError};
use self::contract::scryer::runtime::host as runtime_host;

/// Identifying context for one list component invocation.
pub(crate) struct ListComponentInvocation<'a> {
    pub(crate) plugin_id: &'a str,
    pub(crate) plugin_version: &'a str,
    pub(crate) operation: &'a str,
}

/// Compile-and-link validation for a list component artifact.
pub(crate) fn validate_list_component(wasm: &[u8]) -> Result<(), String> {
    ListComponentRuntime::new(engine::shared_async_engine(), wasm).map(|_| ())
}

/// Store data for one list component invocation.
pub(crate) struct ListComponentCtx {
    table: ResourceTable,
    wasi: WasiCtx,
    limits: HostLimits,
    command_host: CommandHost,
    /// Zero of the monotonic clock reported to the guest; the invocation
    /// deadline is reported on the same timebase.
    clock_origin: Instant,
    deadline: Instant,
}

impl WasiView for ListComponentCtx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl ServicesHost for ListComponentCtx {
    async fn host_call(&mut self, request: Vec<u8>) -> Result<Vec<u8>, HostError> {
        match family_component_host::dispatch_host_call(&self.command_host, request).await {
            Ok(response) => Ok(response),
            Err(HostCallError::Service { failure, error }) => {
                tracing::debug!(
                    target: "scryer_plugins::list",
                    error = error.as_str(),
                    "list component host-call failed",
                );
                Err(match failure {
                    HostCallFailure::InvalidRequest => HostError::InvalidRequest,
                    HostCallFailure::Failed => HostError::Failed,
                })
            }
            Err(HostCallError::Task(error)) => {
                tracing::debug!(
                    target: "scryer_plugins::list",
                    error = %error,
                    "list component host-call task failed",
                );
                Err(HostError::Failed)
            }
        }
    }
}

impl runtime_host::Host for ListComponentCtx {
    async fn monotonic_now_ms(&mut self) -> u64 {
        self.clock_origin
            .elapsed()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }

    async fn operation_deadline_monotonic_ms(&mut self) -> u64 {
        self.deadline
            .saturating_duration_since(self.clock_origin)
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }

    async fn wall_now_ms(&mut self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }

    async fn config_get(&mut self, key: String) -> Option<String> {
        self.command_host.config_get(&key)
    }

    /// List providers carry no provider profile; `none` means "use your
    /// defaults" to shared engine code.
    async fn provider_profile(&mut self) -> Option<Vec<u8>> {
        None
    }

    async fn state_get(&mut self, key: String) -> Option<Vec<u8>> {
        self.command_host.state_get(&key)
    }

    async fn state_cas(
        &mut self,
        key: String,
        expected: Option<Vec<u8>>,
        replacement: Option<Vec<u8>>,
    ) -> bool {
        self.command_host.state_cas(key, expected, replacement)
    }

    async fn log(&mut self, level: runtime_host::LogLevel, message: String) {
        use runtime_host::LogLevel;

        match level {
            LogLevel::Trace => tracing::trace!(target: "scryer_plugins::list", "{message}"),
            LogLevel::Debug => tracing::debug!(target: "scryer_plugins::list", "{message}"),
            LogLevel::Info => tracing::info!(target: "scryer_plugins::list", "{message}"),
            LogLevel::Warn => tracing::warn!(target: "scryer_plugins::list", "{message}"),
            LogLevel::Error => tracing::error!(target: "scryer_plugins::list", "{message}"),
        }
    }
}

impl runtime_host::HostWithStore<ListComponentCtx> for HasSelf<ListComponentCtx> {
    /// One HTTP request through the same service layer the encoded door uses.
    async fn http(
        accessor: &wasmtime::component::Accessor<ListComponentCtx, Self>,
        request: runtime_host::HttpRequest,
    ) -> Result<runtime_host::HttpResponse, runtime_host::TransportError> {
        use runtime_host::{Header, HttpResponse, TransportError};

        let command_host = accessor.with(|mut access| access.get().command_host.clone());
        let encoded = postcard::to_allocvec(&PluginHostRequest::Http(SdkPluginHttpRequest {
            url: request.url,
            method: Some(request.method),
            headers: request
                .headers
                .into_iter()
                .map(|header| (header.name, header.value))
                .collect(),
            body: request.body,
        }))
        .map_err(|_| TransportError::InvalidRequest)?;

        let encoded_response =
            match family_component_host::dispatch_host_call(&command_host, encoded).await {
                Ok(response) => response,
                Err(HostCallError::Service {
                    failure: HostCallFailure::InvalidRequest,
                    error,
                }) => {
                    tracing::debug!(
                        target: "scryer_plugins::list",
                        error = error.as_str(),
                        "list component typed http request was rejected by the host",
                    );
                    return Err(TransportError::InvalidRequest);
                }
                Err(HostCallError::Service { error, .. } | HostCallError::Task(error)) => {
                    tracing::debug!(
                        target: "scryer_plugins::list",
                        error = error.as_str(),
                        "list component typed http request failed in transport",
                    );
                    return Err(TransportError::Transport);
                }
            };

        let PluginHostResponse::Http(result) =
            postcard::from_bytes(&encoded_response).map_err(|_| TransportError::Transport)?
        else {
            return Err(TransportError::Transport);
        };

        match result {
            PluginResult::Ok(response) => Ok(HttpResponse {
                status: response.status,
                headers: response
                    .headers
                    .into_iter()
                    .map(|(name, value)| Header { name, value })
                    .collect(),
                body: response.body,
            }),
            PluginResult::Err(error) => Err(http_transport_error(&error)),
        }
    }

    /// A sleep that cannot outlive the invocation.
    async fn sleep(
        accessor: &wasmtime::component::Accessor<ListComponentCtx, Self>,
        duration_ms: u64,
    ) {
        let deadline = accessor.with(|mut access| access.get().deadline);
        let remaining = deadline.saturating_duration_since(Instant::now());
        tokio::time::sleep(Duration::from_millis(duration_ms).min(remaining)).await;
    }
}

/// Project the service layer's `PluginError` onto `transport-error`, with the
/// same table the subtitle host documents.
fn http_transport_error(error: &PluginError) -> runtime_host::TransportError {
    use runtime_host::TransportError;

    if matches!(error.code, PluginErrorCode::Unsupported) {
        return TransportError::ForbiddenOrigin;
    }
    let detail = error.debug_message.as_deref().unwrap_or_default();
    if detail.contains("is not allowed") {
        TransportError::ForbiddenOrigin
    } else if detail.contains("Invalid URL")
        || detail.contains("only supports GET")
        || detail.contains("is disabled.")
    {
        TransportError::InvalidRequest
    } else if detail.contains("timed out after") || detail.contains("deadline exhausted") {
        TransportError::Timeout
    } else if detail.contains("exceeds the configured maximum number of bytes") {
        TransportError::ResponseTooLarge
    } else {
        TransportError::Transport
    }
}

/// A compiled list component plus its pre-instantiated world binding.
pub(crate) struct ListComponentRuntime {
    component: Arc<Component>,
    instance_pre: contract::ListProviderPre<ListComponentCtx>,
}

impl ListComponentRuntime {
    pub(crate) fn new(engine: &Engine, wasm: &[u8]) -> Result<Self, String> {
        let component = module_cache::list_provider_component(wasm)?;
        if !Engine::same(component.engine(), engine) {
            return Err("list component cache returned an artifact for a different engine".into());
        }
        let mut linker = Linker::new(engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|error| format!("failed to register WASI Preview 2: {error}"))?;
        contract::ListProvider::add_to_linker::<ListComponentCtx, HasSelf<ListComponentCtx>>(
            &mut linker,
            |ctx| ctx,
        )
        .map_err(|error| format!("failed to register list component host: {error:#}"))?;
        let pre = linker
            .instantiate_pre(&component)
            .map_err(|error| format!("failed to preinstantiate list component: {error:#}"))?;
        let instance_pre = contract::ListProviderPre::new(pre).map_err(|error| {
            format!(
                "list component exports are incompatible with scryer:lists/list-provider@1.0.0 ({error:#})"
            )
        })?;
        Ok(Self {
            component,
            instance_pre,
        })
    }

    async fn instantiate(
        &self,
        wasi: WasiCtx,
        command_host: CommandHost,
        memory_max_bytes: Option<usize>,
        timeout: Duration,
    ) -> Result<(Store<ListComponentCtx>, contract::ListProvider), wasmtime::Error> {
        let clock_origin = Instant::now();
        let mut store = Store::new(
            self.component.engine(),
            ListComponentCtx {
                table: ResourceTable::new(),
                wasi,
                limits: HostLimits::new(memory_max_bytes),
                command_host,
                clock_origin,
                deadline: clock_origin + timeout,
            },
        );
        store.limiter(|ctx: &mut ListComponentCtx| &mut ctx.limits);
        store.set_epoch_deadline(engine::deadline_ticks(timeout));
        let plugin = self.instance_pre.instantiate_async(&mut store).await?;
        Ok((store, plugin))
    }
}

/// Extract a descriptor through the world's `describe` export, on a private
/// current-thread runtime so the synchronous loader path can call it from
/// anywhere.
pub(crate) fn list_component_describe(wasm: &[u8]) -> Result<PluginDescriptor, String> {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| {
                        format!("failed to start list component describe runtime: {error}")
                    })?;
                runtime.block_on(describe_async(wasm))
            })
            .join()
            .map_err(|_| "list component describe thread panicked".to_string())?
    })
}

async fn describe_async(wasm: &[u8]) -> Result<PluginDescriptor, String> {
    let runtime = ListComponentRuntime::new(engine::shared_async_engine(), wasm)?;
    let (wasi, stderr) = sandbox::build_component_describe_sandbox();
    let (mut store, plugin) = runtime
        .instantiate(wasi, CommandHost::disabled(), None, DESCRIBE_TIMEOUT)
        .await
        .map_err(|error| format!("failed to instantiate list component for describe: {error:#}"))?;
    let descriptor_json = plugin.call_describe(&mut store).await.map_err(|error| {
        let denied = store.data().limits.memory_denied;
        let failure = error::classify_error(&error, denied);
        let stderr_tail = tail_of(&stderr);
        format!(
            "list component describe failed ({:?}): {}{}",
            failure.kind,
            failure.detail,
            stderr_suffix(&stderr_tail)
        )
    })?;
    serde_json::from_slice::<PluginDescriptor>(&descriptor_json).map_err(|error| {
        format!("list component describe returned invalid PluginDescriptor JSON: {error}")
    })
}

async fn prepare_list_component(
    wasm: Arc<Vec<u8>>,
    timeout: Duration,
) -> Result<ListComponentRuntime, String> {
    let prepare = tokio::task::spawn_blocking(move || {
        ListComponentRuntime::new(engine::shared_async_engine(), &wasm)
    });
    match tokio::time::timeout(timeout, prepare).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(format!("list component preparation task failed: {error}")),
        Err(_) => Err(format!(
            "timed out waiting for list component rehydration after {} ms",
            timeout.as_millis()
        )),
    }
}

/// Instantiate the list component and run one command request→response
/// exchange.
pub(crate) async fn process_list_component(
    spec: &PluginInstanceSpec,
    request: &PluginCommandRequest,
    invocation: ListComponentInvocation<'_>,
) -> AppResult<PluginCommandResponse> {
    let span = tracing::info_span!(
        "list_plugin_invoke",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
    );
    instrumented_list_component(spec, request, invocation)
        .instrument(span)
        .await
}

async fn instrumented_list_component(
    spec: &PluginInstanceSpec,
    request: &PluginCommandRequest,
    invocation: ListComponentInvocation<'_>,
) -> AppResult<PluginCommandResponse> {
    let started = Instant::now();
    let request_bytes = serde_json::to_vec(request).map_err(|error| {
        AppError::Repository(format!("failed to serialize list plugin command: {error}"))
    })?;
    let request_len = request_bytes.len();

    let runtime = prepare_list_component(Arc::clone(&spec.wasm), spec.timeout)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "list provider plugin {}@{} failed to prepare: {error}",
                invocation.plugin_id, invocation.plugin_version
            ))
        })?;

    let PreparedComponentSandbox {
        wasi,
        stdout: _stdout,
        stderr,
        _scratch,
    } = sandbox::build_component_sandbox(&spec.preopens)?;

    let (mut store, plugin) = match runtime
        .instantiate(
            wasi,
            spec.command_host.for_invocation(spec.timeout),
            spec.memory_max_bytes,
            spec.timeout,
        )
        .await
    {
        Ok(instantiated) => instantiated,
        Err(error) => {
            let failure = error::classify_error(&error, false);
            return Err(finish_error(
                &invocation,
                spec.timeout,
                &tail_of(&stderr),
                &failure,
                started,
                request_len,
            ));
        }
    };

    let call_result = store
        .run_concurrent(async move |accessor| plugin.call_process(accessor, request_bytes).await)
        .await
        .and_then(|inner| inner)
        .map(|result| result.map_err(invocation_error_label));
    let denied = store.data().limits.memory_denied;
    let stderr_tail = tail_of(&stderr);

    // Guest stderr can echo request material, so it only ever reaches trace.
    if !stderr_tail.is_empty() {
        tracing::trace!(
            target: "scryer_plugins::list",
            plugin_id = invocation.plugin_id,
            stderr = stderr_tail.as_str(),
            "list plugin stderr",
        );
    }

    let response_bytes = match call_result {
        Ok(Ok(response_bytes)) => response_bytes,
        Ok(Err(label)) => {
            let failure = error::protocol_failure(format!("list component reported {label}"));
            return Err(finish_error(
                &invocation,
                spec.timeout,
                &stderr_tail,
                &failure,
                started,
                request_len,
            ));
        }
        Err(error) => {
            let failure = error::classify_error(&error, denied);
            return Err(finish_error(
                &invocation,
                spec.timeout,
                &stderr_tail,
                &failure,
                started,
                request_len,
            ));
        }
    };

    if denied {
        let failure = error::classify_error(
            &wasmtime::Error::msg("guest exceeded the configured memory cap"),
            true,
        );
        return Err(finish_error(
            &invocation,
            spec.timeout,
            &stderr_tail,
            &failure,
            started,
            request_len,
        ));
    }

    let response: PluginCommandResponse = match serde_json::from_slice(&response_bytes) {
        Ok(response) => response,
        Err(error) => {
            let failure = error::protocol_failure(format!(
                "list component returned invalid PluginCommandResponse JSON: {error}"
            ));
            return Err(finish_error(
                &invocation,
                spec.timeout,
                &stderr_tail,
                &failure,
                started,
                request_len,
            ));
        }
    };

    if response.abi_version != scryer_plugin_sdk::command::COMMAND_ABI_VERSION {
        let failure = error::protocol_failure(format!(
            "list component response used unsupported ABI version {}",
            response.abi_version
        ));
        return Err(finish_error(
            &invocation,
            spec.timeout,
            &stderr_tail,
            &failure,
            started,
            request_len,
        ));
    }

    tracing::debug!(
        target: "scryer_plugins::list",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
        duration_ms = started.elapsed().as_millis() as u64,
        request_bytes = request_len,
        response_bytes = response_bytes.len(),
        disposition = "ok",
        "list plugin invocation complete",
    );

    Ok(response)
}

const fn invocation_error_label(error: InvocationError) -> &'static str {
    match error {
        InvocationError::Failed => "failed",
        InvocationError::Cancelled => "cancelled",
        InvocationError::InvalidResponse => "invalid-response",
    }
}

fn finish_error(
    invocation: &ListComponentInvocation<'_>,
    budget: Duration,
    stderr_tail: &str,
    failure: &error::RunFailure,
    started: Instant,
    request_len: usize,
) -> AppError {
    tracing::debug!(
        target: "scryer_plugins::list",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
        duration_ms = started.elapsed().as_millis() as u64,
        request_bytes = request_len,
        disposition = ?failure.kind,
        "list plugin invocation failed",
    );
    error::to_app_error(
        failure,
        &error::InvocationContext {
            plugin_id: invocation.plugin_id,
            plugin_version: invocation.plugin_version,
            operation: invocation.operation,
            budget,
            stderr_tail,
        },
    )
}

#[cfg(test)]
pub(crate) mod tests;
