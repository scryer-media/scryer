//! WASI Preview 2 host for metadata-provider components.
//!
//! `scryer:metadata/metadata-provider` imports the same two interfaces as the
//! list world (`scryer:host/services@1.0.0` and `scryer:runtime/host@1.0.0`)
//! with the same authority, so this host binds them to the list host's
//! generated interface modules and stores a [`ListComponentCtx`] tagged
//! [`ProviderComponentFamily::Metadata`]. The import implementations live once,
//! in `list_component_host`; this file holds only what differs by world: the
//! generated world binding, `describe` and the `process` driver.
//!
//! Instance-per-request: one `process` call per invocation, then the whole
//! `Store` is dropped.

use std::sync::Arc;
use std::time::{Duration, Instant};

use scryer_application::{AppError, AppResult};
use scryer_plugin_sdk::PluginDescriptor;
use scryer_plugin_sdk::command::{PluginCommandRequest, PluginCommandResponse};
use tracing::Instrument;
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Engine, Store};
use wasmtime_wasi::WasiCtx;

use crate::runtime_backing::PluginInstanceSpec;
use crate::wasmtime_host::command_host::CommandHost;
use crate::wasmtime_host::family_component_host::{DESCRIBE_TIMEOUT, stderr_suffix, tail_of};
use crate::wasmtime_host::list_component_host::{ListComponentCtx, ProviderComponentFamily};
use crate::wasmtime_host::sandbox::{self, PreparedComponentSandbox};
use crate::wasmtime_host::{engine, error, module_cache};

mod contract {
    wasmtime::component::bindgen!({
        world: "scryer:metadata/metadata-provider@1.0.0",
        // Same layout rule as every family world: one canonical copy of each
        // shared package, pushed before the family package that imports it.
        path: ["wit/host-v1.0.0", "wit/runtime-v1.0.0", "wit/metadata-v1.0.0"],
        imports: { default: async },
        exports: { default: async },
        // The shared imports reuse the list world's generated modules, so the
        // list host's implementations answer them for this world too.
        with: {
            "scryer:host/services@1.0.0":
                crate::wasmtime_host::list_component_host::contract::scryer::host::services,
            "scryer:runtime/host@1.0.0":
                crate::wasmtime_host::list_component_host::contract::scryer::runtime::host,
        },
    });
}

use self::contract::InvocationError;

/// Identifying context for one metadata component invocation.
pub(crate) struct MetadataComponentInvocation<'a> {
    pub(crate) plugin_id: &'a str,
    pub(crate) plugin_version: &'a str,
    pub(crate) operation: &'a str,
}

/// Compile-and-link validation for a metadata component artifact.
pub(crate) fn validate_metadata_component(wasm: &[u8]) -> Result<(), String> {
    MetadataComponentRuntime::new(engine::shared_async_engine(), wasm).map(|_| ())
}

/// A compiled metadata component plus its pre-instantiated world binding.
pub(crate) struct MetadataComponentRuntime {
    component: Arc<Component>,
    instance_pre: contract::MetadataProviderPre<ListComponentCtx>,
}

impl MetadataComponentRuntime {
    pub(crate) fn new(engine: &Engine, wasm: &[u8]) -> Result<Self, String> {
        let component = module_cache::metadata_provider_component(wasm)?;
        if !Engine::same(component.engine(), engine) {
            return Err(
                "metadata component cache returned an artifact for a different engine".into(),
            );
        }
        let mut linker = Linker::new(engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|error| format!("failed to register WASI Preview 2: {error}"))?;
        contract::MetadataProvider::add_to_linker::<ListComponentCtx, HasSelf<ListComponentCtx>>(
            &mut linker,
            |ctx| ctx,
        )
        .map_err(|error| format!("failed to register metadata component host: {error:#}"))?;
        let pre = linker
            .instantiate_pre(&component)
            .map_err(|error| format!("failed to preinstantiate metadata component: {error:#}"))?;
        let instance_pre = contract::MetadataProviderPre::new(pre).map_err(|error| {
            format!(
                "metadata component exports are incompatible with scryer:metadata/metadata-provider@1.0.0 ({error:#})"
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
    ) -> Result<(Store<ListComponentCtx>, contract::MetadataProvider), wasmtime::Error> {
        let mut store = Store::new(
            self.component.engine(),
            ListComponentCtx::new(
                ProviderComponentFamily::Metadata,
                wasi,
                command_host,
                memory_max_bytes,
                timeout,
            ),
        );
        store.limiter(|ctx: &mut ListComponentCtx| ctx.limits_mut());
        store.set_epoch_deadline(engine::deadline_ticks(timeout));
        let plugin = self.instance_pre.instantiate_async(&mut store).await?;
        Ok((store, plugin))
    }
}

/// Extract a descriptor through the world's `describe` export, on a private
/// current-thread runtime so the synchronous loader path can call it from
/// anywhere.
pub(crate) fn metadata_component_describe(wasm: &[u8]) -> Result<PluginDescriptor, String> {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| {
                        format!("failed to start metadata component describe runtime: {error}")
                    })?;
                runtime.block_on(describe_async(wasm))
            })
            .join()
            .map_err(|_| "metadata component describe thread panicked".to_string())?
    })
}

async fn describe_async(wasm: &[u8]) -> Result<PluginDescriptor, String> {
    let runtime = MetadataComponentRuntime::new(engine::shared_async_engine(), wasm)?;
    let (wasi, stderr) = sandbox::build_component_describe_sandbox();
    let (mut store, plugin) = runtime
        .instantiate(wasi, CommandHost::disabled(), None, DESCRIBE_TIMEOUT)
        .await
        .map_err(|error| {
            format!("failed to instantiate metadata component for describe: {error:#}")
        })?;
    let descriptor_json = plugin.call_describe(&mut store).await.map_err(|error| {
        let denied = store.data().memory_denied();
        let failure = error::classify_error(&error, denied);
        let stderr_tail = tail_of(&stderr);
        format!(
            "metadata component describe failed ({:?}): {}{}",
            failure.kind,
            failure.detail,
            stderr_suffix(&stderr_tail)
        )
    })?;
    serde_json::from_slice::<PluginDescriptor>(&descriptor_json).map_err(|error| {
        format!("metadata component describe returned invalid PluginDescriptor JSON: {error}")
    })
}

async fn prepare_metadata_component(
    wasm: Arc<Vec<u8>>,
    timeout: Duration,
) -> Result<MetadataComponentRuntime, String> {
    let prepare = tokio::task::spawn_blocking(move || {
        MetadataComponentRuntime::new(engine::shared_async_engine(), &wasm)
    });
    match tokio::time::timeout(timeout, prepare).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(format!(
            "metadata component preparation task failed: {error}"
        )),
        Err(_) => Err(format!(
            "timed out waiting for metadata component rehydration after {} ms",
            timeout.as_millis()
        )),
    }
}

/// Instantiate the metadata component and run one command request→response
/// exchange.
pub(crate) async fn process_metadata_component(
    spec: &PluginInstanceSpec,
    request: &PluginCommandRequest,
    invocation: MetadataComponentInvocation<'_>,
) -> AppResult<PluginCommandResponse> {
    let span = tracing::info_span!(
        "metadata_plugin_invoke",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
    );
    instrumented_metadata_component(spec, request, invocation)
        .instrument(span)
        .await
}

async fn instrumented_metadata_component(
    spec: &PluginInstanceSpec,
    request: &PluginCommandRequest,
    invocation: MetadataComponentInvocation<'_>,
) -> AppResult<PluginCommandResponse> {
    let started = Instant::now();
    let request_bytes = serde_json::to_vec(request).map_err(|error| {
        AppError::Repository(format!(
            "failed to serialize metadata plugin command: {error}"
        ))
    })?;
    let request_len = request_bytes.len();

    let runtime = prepare_metadata_component(Arc::clone(&spec.wasm), spec.timeout)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "metadata provider plugin {}@{} failed to prepare: {error}",
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
    let denied = store.data().memory_denied();
    let stderr_tail = tail_of(&stderr);

    // Guest stderr can echo request material, so it only ever reaches trace.
    if !stderr_tail.is_empty() {
        tracing::trace!(
            target: "scryer_plugins::metadata",
            plugin_id = invocation.plugin_id,
            stderr = stderr_tail.as_str(),
            "metadata plugin stderr",
        );
    }

    let response_bytes = match call_result {
        Ok(Ok(response_bytes)) => response_bytes,
        Ok(Err(label)) => {
            let failure = error::protocol_failure(format!("metadata component reported {label}"));
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
                "metadata component returned invalid PluginCommandResponse JSON: {error}"
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
            "metadata component response used unsupported ABI version {}",
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
        target: "scryer_plugins::metadata",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
        duration_ms = started.elapsed().as_millis() as u64,
        request_bytes = request_len,
        response_bytes = response_bytes.len(),
        disposition = "ok",
        "metadata plugin invocation complete",
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
    invocation: &MetadataComponentInvocation<'_>,
    budget: Duration,
    stderr_tail: &str,
    failure: &error::RunFailure,
    started: Instant,
    request_len: usize,
) -> AppError {
    tracing::debug!(
        target: "scryer_plugins::metadata",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
        duration_ms = started.elapsed().as_millis() as u64,
        request_bytes = request_len,
        disposition = ?failure.kind,
        "metadata plugin invocation failed",
    );
    error::to_app_error(
        failure,
        &error::InvocationContext {
            plugin_kind: error::PluginHostKind::Metadata,
            plugin_id: invocation.plugin_id,
            plugin_version: invocation.plugin_version,
            operation: invocation.operation,
            budget,
            stderr_tail,
        },
    )
}

#[cfg(test)]
mod tests;
