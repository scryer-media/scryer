//! WASI Preview 2 host for download-client components.
//!
//! The subtitle host is this file's template, and the difference between them
//! is only which family package the world comes from: both import the shared
//! `scryer:host/services@1.0.0` interface and both carry the command ABI's
//! [`PluginCommandRequest`]/[`PluginCommandResponse`] JSON envelope on
//! `process`. A download client needs *config, plugin state, HTTP and
//! host-owned archive extraction*, which [`CommandHost`] already implements
//! once, so this host binds `host-call` straight to that service layer.
//!
//! Plugin state is what makes the shared service layer load-bearing for this
//! family rather than merely available: the guest instance is dropped after
//! every `process` call, so a client that authenticates once keeps its session
//! cookie in the host's state map. The [`CommandHost`] handed to
//! [`process_download_client_component`] belongs to the configured client, not
//! to the invocation, so the cookie outlives the instance that stored it —
//! exactly as it does on the wasip1 command path.
//!
//! Instance-per-request, exactly as the command protocol is: one `process` call
//! per plugin invocation, then the whole `Store` is dropped. This family grants
//! no filesystem preopens on any operation, matching the command runtime.

use std::sync::Arc;
use std::time::{Duration, Instant};

use scryer_application::{AppError, AppResult};
use scryer_plugin_sdk::PluginDescriptor;
use scryer_plugin_sdk::command::{PluginCommandRequest, PluginCommandResponse};
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Engine, Store};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::runtime_backing::PluginInstanceSpec;
use crate::wasmtime_host::command_host::CommandHost;
use crate::wasmtime_host::sandbox::{self, HostLimits, PreparedComponentSandbox};
use crate::wasmtime_host::{engine, error, module_cache};

mod contract_v1_0 {
    wasmtime::component::bindgen!({
        world: "scryer:download-client/download-client@1.0.0",
        // Two packages, two paths, host package first — the layout the subtitle
        // world established so `import scryer:host/services@1.0.0` resolves
        // against one canonical copy of `scryer:host` with no `deps/`
        // duplicates and no symlinks to keep in sync.
        path: ["wit/host-v1.0.0", "wit/download-client-v1.0.0"],
        // The WIT signature stays synchronous so a guest needs no async runtime
        // to call it, while the host implementation may await: the service
        // layer reaches HTTP and archive extraction.
        imports: { default: async },
        exports: { default: async },
    });
}

use self::contract_v1_0::InvocationError;
use self::contract_v1_0::scryer::host::services::{Host as ServicesHost, HostError};

/// Describe runs reuse the 10s describe budget of every other backing.
const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Amount of guest stderr forwarded to tracing / attached to error messages.
const STDERR_TAIL_BYTES: usize = 8 * 1024;

/// Identifying context for one download-client component invocation.
pub(crate) struct DownloadClientComponentInvocation<'a> {
    pub(crate) plugin_id: &'a str,
    pub(crate) plugin_version: &'a str,
    pub(crate) operation: &'a str,
}

/// Compile-and-link validation for a download-client component artifact,
/// mirroring `validate_subtitle_component`.
pub(crate) fn validate_download_client_component(wasm: &[u8]) -> Result<(), String> {
    DownloadClientComponentRuntime::new(engine::shared_async_engine(), wasm).map(|_| ())
}

/// Store data for one download-client component invocation.
pub(crate) struct DownloadClientComponentCtx {
    table: ResourceTable,
    wasi: WasiCtx,
    limits: HostLimits,
    command_host: CommandHost,
}

impl WasiView for DownloadClientComponentCtx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl ServicesHost for DownloadClientComponentCtx {
    /// The shared host import, backed by the client's own [`CommandHost`].
    ///
    /// The service layer is synchronous and can block — `ArchiveExtract` drives
    /// a nested extractor invocation through a runtime handle — so it runs on
    /// the blocking pool rather than on this async worker, exactly as the
    /// core-module `scryer_host_call` shim does. A `CommandHost` is a cheap
    /// `Arc` clone, so the move costs nothing and the state map stays shared.
    async fn host_call(&mut self, request: Vec<u8>) -> Result<Vec<u8>, HostError> {
        let command_host = self.command_host.clone();
        let joined = tokio::task::spawn_blocking(move || command_host.call_bytes(&request)).await;
        match joined {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(error)) => {
                // A rejected or undecodable request is the guest's fault and is
                // recoverable by sending a different one; everything else is
                // the transport failing.
                let kind = if error.contains("encoded host request exceeds")
                    || error.contains("invalid postcard host request")
                {
                    HostError::InvalidRequest
                } else {
                    HostError::Failed
                };
                tracing::debug!(
                    target: "scryer_plugins::download_client",
                    error = error.as_str(),
                    "download client component host-call failed",
                );
                Err(kind)
            }
            Err(error) => {
                tracing::debug!(
                    target: "scryer_plugins::download_client",
                    error = %error,
                    "download client component host-call task failed",
                );
                Err(HostError::Failed)
            }
        }
    }
}

/// A compiled download-client component plus its pre-instantiated world
/// binding.
pub(crate) struct DownloadClientComponentRuntime {
    component: Arc<Component>,
    instance_pre: contract_v1_0::DownloadClientPre<DownloadClientComponentCtx>,
}

impl DownloadClientComponentRuntime {
    pub(crate) fn new(engine: &Engine, wasm: &[u8]) -> Result<Self, String> {
        let component = module_cache::download_client_component(wasm)?;
        if !Engine::same(component.engine(), engine) {
            return Err(
                "download client component cache returned an artifact for a different engine"
                    .into(),
            );
        }
        let mut linker = Linker::new(engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|error| format!("failed to register WASI Preview 2: {error}"))?;
        contract_v1_0::DownloadClient::add_to_linker::<
            DownloadClientComponentCtx,
            HasSelf<DownloadClientComponentCtx>,
        >(&mut linker, |ctx| ctx)
        .map_err(|error| format!("failed to register download client component host: {error:#}"))?;
        let raw_instance_pre = linker.instantiate_pre(&component).map_err(|error| {
            format!("failed to preinstantiate download client component: {error:#}")
        })?;
        let instance_pre = contract_v1_0::DownloadClientPre::new(raw_instance_pre).map_err(
            |error| {
                format!(
                    "download client component exports do not match scryer:download-client/download-client@1.0.0: {error:#}"
                )
            },
        )?;
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
    ) -> Result<
        (
            Store<DownloadClientComponentCtx>,
            contract_v1_0::DownloadClient,
        ),
        wasmtime::Error,
    > {
        let mut store = Store::new(
            self.component.engine(),
            DownloadClientComponentCtx {
                table: ResourceTable::new(),
                wasi,
                limits: HostLimits::new(memory_max_bytes),
                command_host,
            },
        );
        store.limiter(|ctx: &mut DownloadClientComponentCtx| &mut ctx.limits);
        store.set_epoch_deadline(engine::deadline_ticks(timeout));
        let plugin = self.instance_pre.instantiate_async(&mut store).await?;
        Ok((store, plugin))
    }
}

/// Extract a descriptor from a download-client component through the world's
/// `describe` export.
///
/// The loader's descriptor path is synchronous while component guests run on
/// the async engine, so the call is driven on a private current-thread runtime
/// on its own thread — safe from inside a Tokio worker (no nested `block_on`)
/// and from a plain thread alike. Describe happens on install and reload, never
/// per invocation.
pub(crate) fn download_client_component_describe(wasm: &[u8]) -> Result<PluginDescriptor, String> {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| {
                        format!(
                            "failed to start download client component describe runtime: {error}"
                        )
                    })?;
                runtime.block_on(describe_async(wasm))
            })
            .join()
            .map_err(|_| "download client component describe thread panicked".to_string())?
    })
}

async fn describe_async(wasm: &[u8]) -> Result<PluginDescriptor, String> {
    let runtime = DownloadClientComponentRuntime::new(engine::shared_async_engine(), wasm)?;
    let (wasi, stderr) = sandbox::build_component_describe_sandbox();
    // Describe is a pure function of the artifact: no services, so a guest that
    // reaches for the host during describe is told `Unsupported` in-band rather
    // than being handed a configured client.
    let (mut store, plugin) = runtime
        .instantiate(wasi, CommandHost::disabled(), None, DESCRIBE_TIMEOUT)
        .await
        .map_err(|error| {
            format!("failed to instantiate download client component for describe: {error:#}")
        })?;
    let descriptor_json = plugin.call_describe(&mut store).await.map_err(|error| {
        let denied = store.data().limits.memory_denied;
        let failure = error::classify_error(&error, denied);
        let stderr_tail = tail_of(&stderr);
        format!(
            "download client component describe failed ({:?}): {}{}",
            failure.kind,
            failure.detail,
            stderr_suffix(&stderr_tail)
        )
    })?;
    serde_json::from_slice::<PluginDescriptor>(&descriptor_json).map_err(|error| {
        format!(
            "download client component describe returned invalid PluginDescriptor JSON: {error}"
        )
    })
}

/// Compile (or reuse) the component off the async worker, with the same
/// preparation timeout the other component paths use.
async fn prepare_download_client_component(
    wasm: Arc<Vec<u8>>,
    timeout: Duration,
) -> Result<DownloadClientComponentRuntime, String> {
    let prepare = tokio::task::spawn_blocking(move || {
        DownloadClientComponentRuntime::new(engine::shared_async_engine(), &wasm)
    });
    match tokio::time::timeout(timeout, prepare).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(format!(
            "download client component preparation task failed: {error}"
        )),
        Err(_) => Err(format!(
            "timed out waiting for download client component rehydration after {} ms",
            timeout.as_millis()
        )),
    }
}

/// Instantiate the download-client component and run one command
/// request→response exchange.
pub(crate) async fn process_download_client_component(
    spec: &PluginInstanceSpec,
    request: &PluginCommandRequest,
    invocation: DownloadClientComponentInvocation<'_>,
) -> AppResult<PluginCommandResponse> {
    let span = tracing::info_span!(
        "download_client_plugin_invoke",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
    );
    let _enter = span.enter();

    let started = Instant::now();
    let request_bytes = serde_json::to_vec(request).map_err(|error| {
        AppError::Repository(format!(
            "failed to serialize download client plugin command: {error}"
        ))
    })?;
    let request_len = request_bytes.len();

    let runtime = prepare_download_client_component(Arc::clone(&spec.wasm), spec.timeout)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "download client plugin {}@{} failed to prepare: {error}",
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

    let call_result = plugin.call_process(&mut store, &request_bytes).await;
    let denied = store.data().limits.memory_denied;
    let stderr_tail = tail_of(&stderr);

    if !stderr_tail.is_empty() {
        tracing::debug!(
            target: "scryer_plugins::download_client",
            plugin_id = invocation.plugin_id,
            stderr = stderr_tail.as_str(),
            "download client plugin stderr",
        );
    }

    let response_bytes = match call_result {
        Ok(Ok(response_bytes)) => response_bytes,
        Ok(Err(invocation_error)) => {
            let failure = error::protocol_failure(format!(
                "download client component reported {}",
                invocation_error_label(invocation_error)
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
                "download client component returned invalid PluginCommandResponse JSON: {error}"
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
            "download client component response used unsupported ABI version {}",
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
        target: "scryer_plugins::download_client",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
        duration_ms = started.elapsed().as_millis() as u64,
        request_bytes = request_len,
        response_bytes = response_bytes.len(),
        disposition = "ok",
        "download client plugin invocation complete",
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
    invocation: &DownloadClientComponentInvocation<'_>,
    budget: Duration,
    stderr_tail: &str,
    failure: &error::RunFailure,
    started: Instant,
    request_len: usize,
) -> AppError {
    tracing::debug!(
        target: "scryer_plugins::download_client",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
        duration_ms = started.elapsed().as_millis() as u64,
        request_bytes = request_len,
        disposition = ?failure.kind,
        "download client plugin invocation failed",
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

fn stderr_suffix(stderr_tail: &str) -> String {
    if stderr_tail.is_empty() {
        String::new()
    } else {
        format!("; guest stderr: {stderr_tail}")
    }
}

/// Size-capped, lossy tail of a captured output pipe.
fn tail_of(pipe: &MemoryOutputPipe) -> String {
    let bytes = pipe.contents();
    let start = bytes.len().saturating_sub(STDERR_TAIL_BYTES);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use scryer_plugin_sdk::PluginResult;
    use scryer_plugin_sdk::command::{
        PluginCommand, PluginCommandResult, PluginDownloadClientCommand,
        PluginDownloadClientCommandResult,
    };
    use scryer_plugin_sdk::host::{
        PluginHostRequest, PluginHostResponse, PluginStateGetRequest, PluginStateGetResponse,
        PluginStateSetRequest,
    };

    /// Guest memory layout for the hand-built fixture component below.
    const DESCRIPTOR_PTR: usize = 0;
    const OK_RESPONSE_PTR: usize = 8192;
    const FAIL_RESPONSE_PTR: usize = 12288;
    const STATE_SET_REQUEST_PTR: usize = 16384;
    const STATE_GET_REQUEST_PTR: usize = 20480;
    const EXPECTED_RESPONSE_PTR: usize = 24576;
    const DESCRIBE_RETURN_PTR: usize = 25600;
    const PROCESS_RETURN_PTR: usize = 25616;
    const HOST_RETURN_PTR: usize = 25632;

    /// The state key/value the fixture round-trips through `host-call`. This is
    /// the cookie-persistence path in miniature: the value exists only in the
    /// host's `CommandHost` state map, so a guest can only produce the expected
    /// response bytes by actually reaching the service layer.
    const FIXTURE_STATE_KEY: &str = "session_cookie";
    pub(crate) const FIXTURE_STATE_VALUE: &str = "fixture-session-cookie";

    /// WAT data-string escaping: every byte as `\xx`.
    fn wat_bytes(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("\\{byte:02x}")).collect()
    }

    fn state_set_request_bytes() -> Vec<u8> {
        postcard::to_allocvec(&PluginHostRequest::StateSet(PluginStateSetRequest {
            key: FIXTURE_STATE_KEY.to_string(),
            value: FIXTURE_STATE_VALUE.as_bytes().to_vec(),
        }))
        .expect("fixture state-set request must encode")
    }

    fn state_get_request_bytes() -> Vec<u8> {
        postcard::to_allocvec(&PluginHostRequest::StateGet(PluginStateGetRequest {
            key: FIXTURE_STATE_KEY.to_string(),
        }))
        .expect("fixture state-get request must encode")
    }

    fn expected_state_get_response_bytes() -> Vec<u8> {
        postcard::to_allocvec(&PluginHostResponse::StateGet(PluginResult::Ok(
            PluginStateGetResponse {
                value: Some(FIXTURE_STATE_VALUE.as_bytes().to_vec()),
            },
        )))
        .expect("fixture state-get response must encode")
    }

    fn download_client_descriptor_json() -> String {
        let descriptor = PluginDescriptor {
            id: "fixture-download-client".to_string(),
            name: "Fixture Download Client".to_string(),
            version: "1.0.0".to_string(),
            sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
            sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
            socket_permissions: Vec::new(),
            provider: scryer_plugin_sdk::ProviderDescriptor::DownloadClient(
                scryer_plugin_sdk::DownloadClientDescriptor {
                    provider_type: "fixture-download-client".to_string(),
                    provider_aliases: Vec::new(),
                    config_fields: Vec::new(),
                    default_base_url: None,
                    allowed_hosts: Vec::new(),
                    accepted_inputs: Vec::new(),
                    isolation_modes: Vec::new(),
                    capabilities: scryer_plugin_sdk::DownloadClientCapabilities::default(),
                },
            ),
        };
        serde_json::to_string(&descriptor).expect("fixture descriptor must serialize")
    }

    /// The document the guest returns once the state round trip verified.
    fn ok_response_json() -> String {
        serde_json::to_string(&PluginCommandResponse::new(
            PluginCommandResult::DownloadClient(PluginDownloadClientCommandResult::TestConnection(
                // The evidence: the guest names the value it could only have
                // read back out of the host's own state map.
                PluginResult::Ok(format!("host-call:{FIXTURE_STATE_VALUE}")),
            )),
        ))
        .expect("fixture ok response must serialize")
    }

    /// The document the guest returns when the round trip did NOT verify.
    fn fail_response_json() -> String {
        serde_json::to_string(&PluginCommandResponse::new(
            PluginCommandResult::DownloadClient(PluginDownloadClientCommandResult::TestConnection(
                PluginResult::Err(scryer_plugin_sdk::PluginError {
                    code: scryer_plugin_sdk::PluginErrorCode::Permanent,
                    public_message: "shared host-call binding mismatch".to_string(),
                    debug_message: None,
                    retry_after_seconds: None,
                    details: None,
                }),
            )),
        ))
        .expect("fixture fail response must serialize")
    }

    /// A minimal but real `scryer:download-client/download-client@1.0.0`
    /// component.
    ///
    /// `describe` returns a static descriptor document. `process` gates its
    /// response on facts that only a correctly wired host can make true: the
    /// request bytes arrived as JSON (first byte is `{`), and a
    /// `scryer:host/services@1.0.0` `StateGet` came back byte-for-byte equal to
    /// the postcard `PluginHostResponse` the service layer must produce for the
    /// value this family persists between invocations. When `state_set` is on
    /// the guest writes that value itself first, which is the within-invocation
    /// round trip; when it is off the guest only reads, which is how the
    /// cross-invocation persistence of a session cookie becomes observable. It
    /// answers with the ok document when the comparison holds and the failure
    /// document otherwise, so the assertion is about the shared binding, not
    /// about a trap.
    fn fixture_component_wat(
        descriptor_json: &str,
        ok_json: &str,
        fail_json: &str,
        state_set_request: &[u8],
        state_get_request: &[u8],
        expected_response: &[u8],
        state_set: bool,
    ) -> String {
        let set_call = if state_set {
            format!(
                r#"      (call $host_call
        (i32.const {set_ptr}) (i32.const {set_len}) (i32.const {host_ret}))
      (if (i32.ne (i32.load8_u (i32.const {host_ret})) (i32.const 0))
        (then (return (call $fail))))"#,
                set_ptr = STATE_SET_REQUEST_PTR,
                set_len = state_set_request.len(),
                host_ret = HOST_RETURN_PTR,
            )
        } else {
            "      ;; read-only guest: no state is written before the read-back".to_string()
        };

        format!(
            r#"(component
  (import "scryer:host/services@1.0.0" (instance $host
    (type (enum "invalid-request" "failed"))
    (export "host-error" (type (eq 0)))
    (export "host-call" (func
      (param "request" (list u8))
      (result (result (list u8) (error 1)))))
  ))

  (type $ie (enum "failed" "cancelled" "invalid-response"))
  (export $ieX "invocation-error" (type $ie))
  (type $describe-ty (func (result (list u8))))
  (type $process-ty (func (param "request" (list u8))
    (result (result (list u8) (error $ieX)))))

  (core module $libc
    (memory (export "memory") 2)
    (global $bump (mut i32) (i32.const 32768))
    (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
      (local $ptr i32)
      (global.set $bump
        (i32.and (i32.add (global.get $bump) (i32.const 7)) (i32.const -8)))
      (local.set $ptr (global.get $bump))
      (global.set $bump (i32.add (global.get $bump) (local.get 3)))
      (local.get $ptr))
  )
  (core instance $libci (instantiate $libc))
  (alias core export $libci "memory" (core memory $mem))
  (alias core export $libci "cabi_realloc" (core func $realloc))

  (core func $host_call_low
    (canon lower (func $host "host-call") (memory $mem) (realloc $realloc)))

  (core module $main
    (import "libc" "memory" (memory 2))
    (import "host" "call" (func $host_call (param i32 i32 i32)))
    (data (i32.const {descriptor_ptr}) "{descriptor}")
    (data (i32.const {ok_ptr}) "{ok}")
    (data (i32.const {fail_ptr}) "{fail}")
    (data (i32.const {set_ptr}) "{set_req}")
    (data (i32.const {get_ptr}) "{get_req}")
    (data (i32.const {expected_ptr}) "{expected}")

    (func $respond (param $ptr i32) (param $len i32) (result i32)
      (i32.store8 (i32.const {process_ret}) (i32.const 0))
      (i32.store (i32.const {process_ret_ptr}) (local.get $ptr))
      (i32.store (i32.const {process_ret_len}) (local.get $len))
      (i32.const {process_ret}))

    (func $fail (result i32)
      (call $respond (i32.const {fail_ptr}) (i32.const {fail_len})))

    (func (export "describe") (result i32)
      (i32.store (i32.const {describe_ret}) (i32.const {descriptor_ptr}))
      (i32.store (i32.const {describe_ret_len}) (i32.const {descriptor_len}))
      (i32.const {describe_ret}))

    (func (export "process") (param $ptr i32) (param $len i32) (result i32)
      (local $index i32)
      (local $response i32)
      ;; The request must have crossed the boundary as JSON.
      (if (i32.eqz (local.get $len)) (then (return (call $fail))))
      (if (i32.ne (i32.load8_u (local.get $ptr)) (i32.const 123))
        (then (return (call $fail))))
      ;; Persist the client's session value, when this fixture writes at all.
{set_call}
      ;; Read it back through the same shared host-call.
      (call $host_call
        (i32.const {get_ptr}) (i32.const {get_len}) (i32.const {host_ret}))
      (if (i32.ne (i32.load8_u (i32.const {host_ret})) (i32.const 0))
        (then (return (call $fail))))
      (if (i32.ne (i32.load (i32.const {host_ret_len})) (i32.const {expected_len}))
        (then (return (call $fail))))
      (local.set $response (i32.load (i32.const {host_ret_ptr})))
      (local.set $index (i32.const 0))
      (block $compared
        (loop $compare
          (br_if $compared (i32.ge_u (local.get $index) (i32.const {expected_len})))
          (if (i32.ne
                (i32.load8_u (i32.add (local.get $response) (local.get $index)))
                (i32.load8_u (i32.add (i32.const {expected_ptr}) (local.get $index))))
            (then (return (call $fail))))
          (local.set $index (i32.add (local.get $index) (i32.const 1)))
          (br $compare)))
      (call $respond (i32.const {ok_ptr}) (i32.const {ok_len})))
  )
  (core instance $maini (instantiate $main
    (with "libc" (instance $libci))
    (with "host" (instance (export "call" (func $host_call_low))))))

  (func (export "describe") (type $describe-ty)
    (canon lift (core func $maini "describe") (memory $mem) (realloc $realloc)))
  (func (export "process") (type $process-ty)
    (canon lift (core func $maini "process") (memory $mem) (realloc $realloc)))
)"#,
            descriptor_ptr = DESCRIPTOR_PTR,
            descriptor = descriptor_json.replace('"', "\\\""),
            descriptor_len = descriptor_json.len(),
            ok_ptr = OK_RESPONSE_PTR,
            ok = ok_json.replace('"', "\\\""),
            ok_len = ok_json.len(),
            fail_ptr = FAIL_RESPONSE_PTR,
            fail = fail_json.replace('"', "\\\""),
            fail_len = fail_json.len(),
            set_ptr = STATE_SET_REQUEST_PTR,
            set_req = wat_bytes(state_set_request),
            get_ptr = STATE_GET_REQUEST_PTR,
            get_req = wat_bytes(state_get_request),
            get_len = state_get_request.len(),
            expected_ptr = EXPECTED_RESPONSE_PTR,
            expected = wat_bytes(expected_response),
            expected_len = expected_response.len(),
            set_call = set_call,
            describe_ret = DESCRIBE_RETURN_PTR,
            describe_ret_len = DESCRIBE_RETURN_PTR + 4,
            process_ret = PROCESS_RETURN_PTR,
            process_ret_ptr = PROCESS_RETURN_PTR + 4,
            process_ret_len = PROCESS_RETURN_PTR + 8,
            host_ret = HOST_RETURN_PTR,
            host_ret_ptr = HOST_RETURN_PTR + 4,
            host_ret_len = HOST_RETURN_PTR + 8,
        )
    }

    /// The fixture that writes its session value and reads it straight back.
    pub(crate) fn fixture_component() -> Vec<u8> {
        wat::parse_str(fixture_component_wat(
            &download_client_descriptor_json(),
            &ok_response_json(),
            &fail_response_json(),
            &state_set_request_bytes(),
            &state_get_request_bytes(),
            &expected_state_get_response_bytes(),
            true,
        ))
        .expect("fixture download client component WAT must assemble")
    }

    /// The fixture that only reads: it succeeds exactly when a *previous*
    /// invocation left the value in the client's host state.
    fn read_only_fixture_component() -> Vec<u8> {
        wat::parse_str(fixture_component_wat(
            &download_client_descriptor_json(),
            &ok_response_json(),
            &fail_response_json(),
            &state_set_request_bytes(),
            &state_get_request_bytes(),
            &expected_state_get_response_bytes(),
            false,
        ))
        .expect("fixture download client component WAT must assemble")
    }

    /// A fixture built against a host response the service layer will never
    /// produce, so the guest takes its failure branch. This is what makes the
    /// round-trip assertion meaningful rather than incidental.
    fn mismatched_fixture_component() -> Vec<u8> {
        let mut expected = expected_state_get_response_bytes();
        expected.push(0xff);
        wat::parse_str(fixture_component_wat(
            &download_client_descriptor_json(),
            &ok_response_json(),
            &fail_response_json(),
            &state_set_request_bytes(),
            &state_get_request_bytes(),
            &expected,
            true,
        ))
        .expect("fixture download client component WAT must assemble")
    }

    fn configured_command_host() -> CommandHost {
        CommandHost::with_archive_provider(
            "fixture-download-client".to_string(),
            BTreeMap::new(),
            Vec::new(),
            Duration::from_secs(30),
            None,
            None,
        )
    }

    fn test_spec(wasm: Vec<u8>, command_host: CommandHost) -> PluginInstanceSpec {
        PluginInstanceSpec {
            wasm: Arc::new(wasm),
            preopens: Vec::new(),
            timeout: Duration::from_secs(30),
            memory_max_bytes: None,
            command_host,
        }
    }

    fn test_connection_request() -> PluginCommandRequest {
        PluginCommandRequest::new(PluginCommand::DownloadClient(
            PluginDownloadClientCommand::TestConnection,
        ))
    }

    fn invocation() -> DownloadClientComponentInvocation<'static> {
        DownloadClientComponentInvocation {
            plugin_id: "fixture-download-client",
            plugin_version: "1.0.0",
            operation: "test_connection",
        }
    }

    fn test_connection_result(response: PluginCommandResponse) -> PluginResult<String> {
        let PluginCommandResult::DownloadClient(PluginDownloadClientCommandResult::TestConnection(
            result,
        )) = response.response
        else {
            panic!("fixture must answer a test-connection command with a test-connection result");
        };
        result
    }

    #[test]
    fn a_core_module_download_client_artifact_fails_world_validation() {
        let core_module = wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "_start")))"#,
        )
        .expect("core module WAT must parse");

        let error = validate_download_client_component(&core_module)
            .expect_err("a core module must not validate as a download client component");
        assert!(
            error.contains("component") || error.contains("compile"),
            "{error}"
        );
    }

    #[test]
    fn an_arbitrary_component_fails_world_validation() {
        let wasm = wat::parse_str("(component)").expect("component WAT must parse");

        let error = validate_download_client_component(&wasm)
            .expect_err("an arbitrary component must not pass download-client-world validation");
        assert!(error.contains("exports do not match"), "{error}");
    }

    #[test]
    fn the_fixture_component_passes_world_validation() {
        validate_download_client_component(&fixture_component())
            .expect("the fixture must satisfy scryer:download-client/download-client@1.0.0");
    }

    /// The archive world exports the same two functions and is told apart only
    /// by what it imports, so a download-client component must not silently
    /// link as an archive extractor.
    #[test]
    fn a_download_client_component_does_not_validate_as_an_archive_component() {
        let error = crate::wasmtime_host::validate_archive_component(&fixture_component())
            .expect_err("a download client component must not satisfy the archive world");
        assert!(!error.is_empty(), "{error}");
    }

    /// The subtitle and download-client worlds import the same shared services
    /// interface and export the same two functions, so at the component-type
    /// level they are the *same* world and each validates as the other. That is
    /// not a gap: family separation is the descriptor's job — the loader picks
    /// a backing from `PluginDescriptor::provider` — and this test pins the
    /// property so a future world change does not silently start relying on
    /// structural separation that was never there.
    #[test]
    fn the_subtitle_world_is_structurally_the_same_world() {
        let subtitle = crate::wasmtime_host::subtitle_component_host::tests::fixture_component();

        validate_download_client_component(&subtitle)
            .expect("the two families share a world shape; only the descriptor tells them apart");
        crate::wasmtime_host::validate_subtitle_component(&fixture_component())
            .expect("and symmetrically");
    }

    #[test]
    fn describe_returns_the_guest_descriptor() {
        let descriptor = download_client_component_describe(&fixture_component())
            .expect("the fixture must self-describe through the world's describe export");

        assert_eq!(descriptor.id, "fixture-download-client");
        assert!(matches!(
            descriptor.provider,
            scryer_plugin_sdk::ProviderDescriptor::DownloadClient(_)
        ));
    }

    /// The end-to-end host path: the command envelope crosses as JSON, the
    /// shared `scryer:host/services@1.0.0` import is callable and reaches the
    /// client's own `CommandHost` state, and the response deserializes into the
    /// SDK command types.
    #[tokio::test(flavor = "multi_thread")]
    async fn process_round_trips_the_command_envelope_through_the_shared_host_call() {
        let spec = test_spec(fixture_component(), configured_command_host());

        let response =
            process_download_client_component(&spec, &test_connection_request(), invocation())
                .await
                .expect("the fixture component must complete one process exchange");

        let PluginResult::Ok(message) = test_connection_result(response) else {
            panic!("a plugin error means the shared host-call did not round-trip intact");
        };
        assert_eq!(
            message,
            format!("host-call:{FIXTURE_STATE_VALUE}"),
            "the guest must have read the persisted value back through host-call",
        );
    }

    /// The cookie-persistence contract: the state a client writes during one
    /// `process` call is still there for the next one, even though the guest
    /// instance in between was dropped. A read-only guest sees nothing before
    /// the writing guest has run and sees the value afterwards.
    #[tokio::test(flavor = "multi_thread")]
    async fn client_state_outlives_the_instance_that_stored_it() {
        let command_host = configured_command_host();
        let reader = test_spec(read_only_fixture_component(), command_host.clone());
        let writer = test_spec(fixture_component(), command_host.clone());

        let before =
            process_download_client_component(&reader, &test_connection_request(), invocation())
                .await
                .expect("a read before any write must still complete the exchange");
        assert!(
            matches!(test_connection_result(before), PluginResult::Err(_)),
            "an empty state map must not look like a stored session value",
        );

        process_download_client_component(&writer, &test_connection_request(), invocation())
            .await
            .expect("the writing fixture must complete its exchange");

        let after =
            process_download_client_component(&reader, &test_connection_request(), invocation())
                .await
                .expect("the read-back invocation must complete");
        assert!(
            matches!(test_connection_result(after), PluginResult::Ok(_)),
            "state written by an earlier invocation must survive into the next one",
        );
    }

    /// A guest whose expectation of the host response is wrong takes its
    /// failure branch — proof that the success above is the round trip and not
    /// a guest that ignores the host.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_mismatched_host_response_drives_the_guest_failure_branch() {
        let spec = test_spec(mismatched_fixture_component(), configured_command_host());

        let response =
            process_download_client_component(&spec, &test_connection_request(), invocation())
                .await
                .expect("the fixture component must still complete the exchange");

        let PluginResult::Err(error) = test_connection_result(response) else {
            panic!("a mismatched host response must not produce a successful exchange");
        };
        assert!(
            error.public_message.contains("host-call binding mismatch"),
            "{}",
            error.public_message
        );
    }

    /// A host with no services configured answers in-band with `Unsupported`
    /// rather than failing the transport, so the guest sees different bytes and
    /// takes its failure branch — the capability-availability contract the WIT
    /// documents.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_disabled_host_reports_unsupported_in_band() {
        let spec = test_spec(fixture_component(), CommandHost::disabled());

        let response =
            process_download_client_component(&spec, &test_connection_request(), invocation())
                .await
                .expect("a disabled host must not fail the invocation itself");

        assert!(
            matches!(test_connection_result(response), PluginResult::Err(_)),
            "an unconfigured service must not look like a stored session value",
        );
    }

    /// A `process` response that is not a `PluginCommandResponse` is a protocol
    /// failure, not a silent empty result.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_non_json_process_response_is_a_protocol_failure() {
        let wasm = wat::parse_str(fixture_component_wat(
            &download_client_descriptor_json(),
            "not json at all",
            "not json either",
            &state_set_request_bytes(),
            &state_get_request_bytes(),
            &expected_state_get_response_bytes(),
            true,
        ))
        .expect("fixture download client component WAT must assemble");
        let spec = test_spec(wasm, configured_command_host());

        let error =
            process_download_client_component(&spec, &test_connection_request(), invocation())
                .await
                .expect_err("a malformed response document must fail the invocation");

        assert!(
            error.to_string().contains("PluginCommandResponse"),
            "{error}"
        );
    }
}
