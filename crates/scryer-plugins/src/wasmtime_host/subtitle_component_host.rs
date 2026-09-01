//! WASI Preview 2 host for subtitle-provider components.
//!
//! The archive host is this file's template; the difference is what the world
//! imports. An archive extractor imports family-specific crypto because it is
//! the one family that needs raw AES/CRC cores. Every other family — subtitles
//! first, download clients and notifications next — needs *config, state, HTTP
//! and host-owned archive extraction*, which are already implemented once in
//! [`CommandHost`]. So this host binds the shared
//! `scryer:host/services@1.0.0` import straight to that service layer instead
//! of growing a second copy of it, and the WIT payload is the SDK's postcard
//! `PluginHostRequest`/`PluginHostResponse` — the same bytes the core-module
//! `scryer_host_call` ABI carries today.
//!
//! The `process` payload is likewise not new: it is the wasip1 command ABI's
//! [`PluginCommandRequest`]/[`PluginCommandResponse`] JSON envelope, moved from
//! stdin/stdout onto the world's `process` export. A command-ABI subtitle guest
//! migrates by changing its transport, not its types.
//!
//! Instance-per-request, exactly as the command protocol is: one `process` call
//! per plugin invocation, then the whole `Store` is dropped.

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
        world: "scryer:subtitle/subtitle-provider@1.0.0",
        // Two packages, two paths. The shared host package is pushed first so
        // the family package's `import scryer:host/services@1.0.0` resolves
        // against it. This is the layout every following family world uses —
        // one canonical copy of `scryer:host`, no `deps/` duplicates and no
        // symlinks to keep in sync.
        path: ["wit/host-v1.0.0", "wit/subtitle-v1.0.0"],
        // The WIT signature stays synchronous so a guest needs no async
        // runtime to call it, while the host implementation may await: the
        // service layer reaches HTTP and archive extraction.
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

/// Identifying context for one subtitle component invocation.
pub(crate) struct SubtitleComponentInvocation<'a> {
    pub(crate) plugin_id: &'a str,
    pub(crate) plugin_version: &'a str,
    pub(crate) operation: &'a str,
}

/// Compile-and-link validation for a subtitle component artifact, mirroring
/// `validate_archive_component`.
pub(crate) fn validate_subtitle_component(wasm: &[u8]) -> Result<(), String> {
    SubtitleComponentRuntime::new(engine::shared_async_engine(), wasm).map(|_| ())
}

/// Store data for one subtitle component invocation.
pub(crate) struct SubtitleComponentCtx {
    table: ResourceTable,
    wasi: WasiCtx,
    limits: HostLimits,
    command_host: CommandHost,
}

impl WasiView for SubtitleComponentCtx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl ServicesHost for SubtitleComponentCtx {
    /// The shared host import, backed by the plugin's own [`CommandHost`].
    ///
    /// The service layer is synchronous and can block — `ArchiveExtract` drives
    /// a nested extractor invocation through a runtime handle — so it runs on
    /// the blocking pool rather than on this async worker, exactly as the
    /// core-module `scryer_host_call` shim does. A `CommandHost` is a cheap
    /// `Arc` clone, so the move costs nothing.
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
                    target: "scryer_plugins::subtitle",
                    error = error.as_str(),
                    "subtitle component host-call failed",
                );
                Err(kind)
            }
            Err(error) => {
                tracing::debug!(
                    target: "scryer_plugins::subtitle",
                    error = %error,
                    "subtitle component host-call task failed",
                );
                Err(HostError::Failed)
            }
        }
    }
}

/// A compiled subtitle component plus its pre-instantiated world binding.
pub(crate) struct SubtitleComponentRuntime {
    component: Arc<Component>,
    instance_pre: contract_v1_0::SubtitleProviderPre<SubtitleComponentCtx>,
}

impl SubtitleComponentRuntime {
    pub(crate) fn new(engine: &Engine, wasm: &[u8]) -> Result<Self, String> {
        let component = module_cache::subtitle_component(wasm)?;
        if !Engine::same(component.engine(), engine) {
            return Err(
                "subtitle component cache returned an artifact for a different engine".into(),
            );
        }
        let mut linker = Linker::new(engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|error| format!("failed to register WASI Preview 2: {error}"))?;
        contract_v1_0::SubtitleProvider::add_to_linker::<
            SubtitleComponentCtx,
            HasSelf<SubtitleComponentCtx>,
        >(&mut linker, |ctx| ctx)
        .map_err(|error| format!("failed to register subtitle component host: {error:#}"))?;
        let raw_instance_pre = linker
            .instantiate_pre(&component)
            .map_err(|error| format!("failed to preinstantiate subtitle component: {error:#}"))?;
        let instance_pre = contract_v1_0::SubtitleProviderPre::new(raw_instance_pre).map_err(
            |error| {
                format!(
                    "subtitle component exports do not match scryer:subtitle/subtitle-provider@1.0.0: {error:#}"
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
            Store<SubtitleComponentCtx>,
            contract_v1_0::SubtitleProvider,
        ),
        wasmtime::Error,
    > {
        let mut store = Store::new(
            self.component.engine(),
            SubtitleComponentCtx {
                table: ResourceTable::new(),
                wasi,
                limits: HostLimits::new(memory_max_bytes),
                command_host,
            },
        );
        store.limiter(|ctx: &mut SubtitleComponentCtx| &mut ctx.limits);
        store.set_epoch_deadline(engine::deadline_ticks(timeout));
        let plugin = self.instance_pre.instantiate_async(&mut store).await?;
        Ok((store, plugin))
    }
}

/// Extract a descriptor from a subtitle component through the world's
/// `describe` export.
///
/// The loader's descriptor path is synchronous while component guests run on
/// the async engine, so the call is driven on a private current-thread runtime
/// on its own thread — safe from inside a Tokio worker (no nested `block_on`)
/// and from a plain thread alike. Describe happens on install and reload, never
/// per invocation.
pub(crate) fn subtitle_component_describe(wasm: &[u8]) -> Result<PluginDescriptor, String> {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| {
                        format!("failed to start subtitle component describe runtime: {error}")
                    })?;
                runtime.block_on(describe_async(wasm))
            })
            .join()
            .map_err(|_| "subtitle component describe thread panicked".to_string())?
    })
}

async fn describe_async(wasm: &[u8]) -> Result<PluginDescriptor, String> {
    let runtime = SubtitleComponentRuntime::new(engine::shared_async_engine(), wasm)?;
    let (wasi, stderr) = sandbox::build_component_describe_sandbox();
    // Describe is a pure function of the artifact: no services, so a guest that
    // reaches for the host during describe is told `Unsupported` in-band rather
    // than being handed a configured provider.
    let (mut store, plugin) = runtime
        .instantiate(wasi, CommandHost::disabled(), None, DESCRIBE_TIMEOUT)
        .await
        .map_err(|error| {
            format!("failed to instantiate subtitle component for describe: {error:#}")
        })?;
    let descriptor_json = plugin.call_describe(&mut store).await.map_err(|error| {
        let denied = store.data().limits.memory_denied;
        let failure = error::classify_error(&error, denied);
        let stderr_tail = tail_of(&stderr);
        format!(
            "subtitle component describe failed ({:?}): {}{}",
            failure.kind,
            failure.detail,
            stderr_suffix(&stderr_tail)
        )
    })?;
    serde_json::from_slice::<PluginDescriptor>(&descriptor_json).map_err(|error| {
        format!("subtitle component describe returned invalid PluginDescriptor JSON: {error}")
    })
}

/// Compile (or reuse) the component off the async worker, with the same
/// preparation timeout the archive component path uses.
async fn prepare_subtitle_component(
    wasm: Arc<Vec<u8>>,
    timeout: Duration,
) -> Result<SubtitleComponentRuntime, String> {
    let prepare = tokio::task::spawn_blocking(move || {
        SubtitleComponentRuntime::new(engine::shared_async_engine(), &wasm)
    });
    match tokio::time::timeout(timeout, prepare).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(format!(
            "subtitle component preparation task failed: {error}"
        )),
        Err(_) => Err(format!(
            "timed out waiting for subtitle component rehydration after {} ms",
            timeout.as_millis()
        )),
    }
}

/// Instantiate the subtitle component and run one command request→response
/// exchange.
pub(crate) async fn process_subtitle_component(
    spec: &PluginInstanceSpec,
    request: &PluginCommandRequest,
    invocation: SubtitleComponentInvocation<'_>,
) -> AppResult<PluginCommandResponse> {
    let span = tracing::info_span!(
        "subtitle_plugin_invoke",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
    );
    let _enter = span.enter();

    let started = Instant::now();
    let request_bytes = serde_json::to_vec(request).map_err(|error| {
        AppError::Repository(format!(
            "failed to serialize subtitle plugin command: {error}"
        ))
    })?;
    let request_len = request_bytes.len();

    let runtime = prepare_subtitle_component(Arc::clone(&spec.wasm), spec.timeout)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "subtitle provider plugin {}@{} failed to prepare: {error}",
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
            target: "scryer_plugins::subtitle",
            plugin_id = invocation.plugin_id,
            stderr = stderr_tail.as_str(),
            "subtitle plugin stderr",
        );
    }

    let response_bytes = match call_result {
        Ok(Ok(response_bytes)) => response_bytes,
        Ok(Err(invocation_error)) => {
            let failure = error::protocol_failure(format!(
                "subtitle component reported {}",
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
                "subtitle component returned invalid PluginCommandResponse JSON: {error}"
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
            "subtitle component response used unsupported ABI version {}",
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
        target: "scryer_plugins::subtitle",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
        duration_ms = started.elapsed().as_millis() as u64,
        request_bytes = request_len,
        response_bytes = response_bytes.len(),
        disposition = "ok",
        "subtitle plugin invocation complete",
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
    invocation: &SubtitleComponentInvocation<'_>,
    budget: Duration,
    stderr_tail: &str,
    failure: &error::RunFailure,
    started: Instant,
    request_len: usize,
) -> AppError {
    tracing::debug!(
        target: "scryer_plugins::subtitle",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
        duration_ms = started.elapsed().as_millis() as u64,
        request_bytes = request_len,
        disposition = ?failure.kind,
        "subtitle plugin invocation failed",
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

    use scryer_plugin_sdk::command::{
        PluginCommand, PluginCommandResult, PluginSubtitleCommand, PluginSubtitleCommandResult,
    };
    use scryer_plugin_sdk::host::{
        PluginConfigGetRequest, PluginConfigGetResponse, PluginHostRequest, PluginHostResponse,
    };
    use scryer_plugin_sdk::{
        PluginResult, SubtitlePluginCandidate, SubtitlePluginSearchRequest,
        SubtitlePluginSearchResponse, SubtitleQueryMediaKind,
    };

    /// Guest memory layout for the hand-built fixture component below.
    const DESCRIPTOR_PTR: usize = 0;
    const OK_RESPONSE_PTR: usize = 8192;
    const FAIL_RESPONSE_PTR: usize = 16384;
    const HOST_REQUEST_PTR: usize = 24576;
    const EXPECTED_RESPONSE_PTR: usize = 24832;
    const DESCRIBE_RETURN_PTR: usize = 25600;
    const PROCESS_RETURN_PTR: usize = 25616;
    const HOST_RETURN_PTR: usize = 25632;

    /// The config key/value the fixture round-trips through `host-call`. The
    /// value only exists in the host's `CommandHost` config, so a guest can
    /// only produce the expected response bytes by actually reaching the
    /// service layer.
    const FIXTURE_CONFIG_KEY: &str = "api_key";
    const FIXTURE_CONFIG_VALUE: &str = "fixture-host-call-secret";

    /// WAT data-string escaping: every byte as `\xx`.
    fn wat_bytes(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("\\{byte:02x}")).collect()
    }

    fn host_request_bytes() -> Vec<u8> {
        postcard::to_allocvec(&PluginHostRequest::ConfigGet(PluginConfigGetRequest {
            key: FIXTURE_CONFIG_KEY.to_string(),
        }))
        .expect("fixture host request must encode")
    }

    fn expected_host_response_bytes() -> Vec<u8> {
        postcard::to_allocvec(&PluginHostResponse::ConfigGet(PluginResult::Ok(
            PluginConfigGetResponse {
                value: Some(FIXTURE_CONFIG_VALUE.to_string()),
            },
        )))
        .expect("fixture host response must encode")
    }

    fn subtitle_descriptor_json() -> String {
        let descriptor = PluginDescriptor {
            id: "fixture-subtitle".to_string(),
            name: "Fixture Subtitle".to_string(),
            version: "1.0.0".to_string(),
            sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
            sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
            socket_permissions: Vec::new(),
            provider: scryer_plugin_sdk::ProviderDescriptor::Subtitle(
                scryer_plugin_sdk::SubtitleDescriptor {
                    provider_type: "fixture-subtitles".to_string(),
                    provider_aliases: Vec::new(),
                    config_fields: Vec::new(),
                    default_base_url: None,
                    allowed_hosts: Vec::new(),
                    capabilities: scryer_plugin_sdk::SubtitleCapabilities::default(),
                },
            ),
        };
        serde_json::to_string(&descriptor).expect("fixture descriptor must serialize")
    }

    /// The document the guest returns once the host-call round trip verified.
    fn ok_response_json() -> String {
        serde_json::to_string(&PluginCommandResponse::new(PluginCommandResult::Subtitle(
            PluginSubtitleCommandResult::Search(PluginResult::Ok(SubtitlePluginSearchResponse {
                results: vec![SubtitlePluginCandidate {
                    // The evidence: the guest names the value it could only
                    // have learned by calling the host.
                    provider_file_id: format!("host-call:{FIXTURE_CONFIG_VALUE}"),
                    language: "eng".to_string(),
                    release_info: None,
                    hearing_impaired: false,
                    forced: false,
                    ai_translated: false,
                    machine_translated: false,
                    uploader: None,
                    download_count: None,
                    match_hints: Vec::new(),
                }],
            })),
        )))
        .expect("fixture ok response must serialize")
    }

    /// The document the guest returns when the round trip did NOT verify.
    fn fail_response_json() -> String {
        serde_json::to_string(&PluginCommandResponse::new(PluginCommandResult::Subtitle(
            PluginSubtitleCommandResult::Search(PluginResult::Err(
                scryer_plugin_sdk::PluginError {
                    code: scryer_plugin_sdk::PluginErrorCode::Permanent,
                    public_message: "shared host-call binding mismatch".to_string(),
                    debug_message: None,
                    retry_after_seconds: None,
                    details: None,
                },
            )),
        )))
        .expect("fixture fail response must serialize")
    }

    /// A minimal but real `scryer:subtitle/subtitle-provider@1.0.0` component.
    ///
    /// `describe` returns a static descriptor document. `process` gates its
    /// response on two facts that only a correctly wired host can make true:
    /// the request bytes arrived as JSON (first byte is `{`), and one
    /// `scryer:host/services@1.0.0` `host-call` carrying a postcard
    /// `PluginHostRequest::ConfigGet` came back byte-for-byte equal to the
    /// postcard `PluginHostResponse` the service layer must produce for this
    /// plugin's configured value. It answers with the ok document when both
    /// hold and the failure document otherwise, so the assertion is about the
    /// shared binding, not about a trap.
    fn fixture_component_wat(
        descriptor_json: &str,
        ok_json: &str,
        fail_json: &str,
        host_request: &[u8],
        expected_response: &[u8],
    ) -> String {
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
    (data (i32.const {host_req_ptr}) "{host_req}")
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
      ;; One shared host-call round trip.
      (call $host_call
        (i32.const {host_req_ptr}) (i32.const {host_req_len}) (i32.const {host_ret}))
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
            host_req_ptr = HOST_REQUEST_PTR,
            host_req = wat_bytes(host_request),
            host_req_len = host_request.len(),
            expected_ptr = EXPECTED_RESPONSE_PTR,
            expected = wat_bytes(expected_response),
            expected_len = expected_response.len(),
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

    pub(crate) fn fixture_component() -> Vec<u8> {
        wat::parse_str(fixture_component_wat(
            &subtitle_descriptor_json(),
            &ok_response_json(),
            &fail_response_json(),
            &host_request_bytes(),
            &expected_host_response_bytes(),
        ))
        .expect("fixture subtitle component WAT must assemble")
    }

    /// A fixture built against a host response the service layer will never
    /// produce, so the guest takes its failure branch. This is what makes the
    /// round-trip assertion meaningful rather than incidental.
    fn mismatched_fixture_component() -> Vec<u8> {
        let mut expected = expected_host_response_bytes();
        expected.push(0xff);
        wat::parse_str(fixture_component_wat(
            &subtitle_descriptor_json(),
            &ok_response_json(),
            &fail_response_json(),
            &host_request_bytes(),
            &expected,
        ))
        .expect("fixture subtitle component WAT must assemble")
    }

    fn configured_command_host() -> CommandHost {
        let mut config = BTreeMap::new();
        config.insert(
            FIXTURE_CONFIG_KEY.to_string(),
            FIXTURE_CONFIG_VALUE.to_string(),
        );
        CommandHost::with_archive_provider(
            "fixture-subtitle".to_string(),
            config,
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

    fn search_command_request() -> PluginCommandRequest {
        PluginCommandRequest::new(PluginCommand::Subtitle(PluginSubtitleCommand::Search(
            SubtitlePluginSearchRequest {
                media_kind: SubtitleQueryMediaKind::Movie,
                facet: None,
                file_hash: None,
                imdb_id: None,
                series_imdb_id: None,
                title: "Fixture Movie".to_string(),
                title_aliases: Vec::new(),
                title_candidates: Vec::new(),
                year: Some(2026),
                season: None,
                episode: None,
                absolute_episode: None,
                external_ids: Default::default(),
                languages: vec!["eng".to_string()],
                release_group: None,
                source: None,
                video_codec: None,
                audio_codec: None,
                resolution: None,
                hearing_impaired: None,
                include_ai_translated: false,
                include_machine_translated: false,
            },
        )))
    }

    fn invocation() -> SubtitleComponentInvocation<'static> {
        SubtitleComponentInvocation {
            plugin_id: "fixture-subtitle",
            plugin_version: "1.0.0",
            operation: "Search",
        }
    }

    fn search_result(response: PluginCommandResponse) -> PluginResult<SubtitlePluginSearchResponse> {
        let PluginCommandResult::Subtitle(PluginSubtitleCommandResult::Search(result)) =
            response.response
        else {
            panic!("fixture must answer a search command with a search result");
        };
        result
    }

    #[test]
    fn a_core_module_subtitle_artifact_fails_world_validation() {
        let core_module = wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "_start")))"#,
        )
        .expect("core module WAT must parse");

        let error = validate_subtitle_component(&core_module)
            .expect_err("a core module must not validate as a subtitle component");
        assert!(
            error.contains("component") || error.contains("compile"),
            "{error}"
        );
    }

    #[test]
    fn an_arbitrary_component_fails_world_validation() {
        let wasm = wat::parse_str("(component)").expect("component WAT must parse");

        let error = validate_subtitle_component(&wasm)
            .expect_err("an arbitrary component must not pass subtitle-world validation");
        assert!(error.contains("exports do not match"), "{error}");
    }

    #[test]
    fn the_fixture_component_passes_world_validation() {
        validate_subtitle_component(&fixture_component())
            .expect("the fixture must satisfy scryer:subtitle/subtitle-provider@1.0.0");
    }

    /// The archive world and the subtitle world export the same two functions
    /// and are told apart only by what they import, so a subtitle component
    /// must not silently link as an archive extractor.
    #[test]
    fn a_subtitle_component_does_not_validate_as_an_archive_component() {
        let error = crate::wasmtime_host::validate_archive_component(&fixture_component())
            .expect_err("a subtitle component must not satisfy the archive world");
        assert!(!error.is_empty(), "{error}");
    }

    #[test]
    fn describe_returns_the_guest_descriptor() {
        let descriptor = subtitle_component_describe(&fixture_component())
            .expect("the fixture must self-describe through the world's describe export");

        assert_eq!(descriptor.id, "fixture-subtitle");
        assert!(matches!(
            descriptor.provider,
            scryer_plugin_sdk::ProviderDescriptor::Subtitle(_)
        ));
    }

    /// The end-to-end host path: the command envelope crosses as JSON, the
    /// shared `scryer:host/services@1.0.0` import is callable and reaches the
    /// plugin's own `CommandHost` config, and the response deserializes into
    /// the SDK command types.
    #[tokio::test(flavor = "multi_thread")]
    async fn process_round_trips_the_command_envelope_through_the_shared_host_call() {
        let spec = test_spec(fixture_component(), configured_command_host());

        let response = process_subtitle_component(&spec, &search_command_request(), invocation())
            .await
            .expect("the fixture component must complete one process exchange");

        let PluginResult::Ok(search) = search_result(response) else {
            panic!("a plugin error means the shared host-call did not round-trip intact");
        };
        assert_eq!(search.results.len(), 1);
        assert_eq!(
            search.results[0].provider_file_id,
            format!("host-call:{FIXTURE_CONFIG_VALUE}"),
            "the guest must have learned the configured value through host-call",
        );
    }

    /// A guest whose expectation of the host response is wrong takes its
    /// failure branch — proof that the success above is the round trip and not
    /// a guest that ignores the host.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_mismatched_host_response_drives_the_guest_failure_branch() {
        let spec = test_spec(mismatched_fixture_component(), configured_command_host());

        let response = process_subtitle_component(&spec, &search_command_request(), invocation())
            .await
            .expect("the fixture component must still complete the exchange");

        let PluginResult::Err(error) = search_result(response) else {
            panic!("a mismatched host response must not produce a successful search");
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

        let response = process_subtitle_component(&spec, &search_command_request(), invocation())
            .await
            .expect("a disabled host must not fail the invocation itself");

        assert!(
            matches!(search_result(response), PluginResult::Err(_)),
            "an unconfigured service must not look like a successful lookup",
        );
    }

    /// A `process` response that is not a `PluginCommandResponse` is a protocol
    /// failure, not a silent empty result.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_non_json_process_response_is_a_protocol_failure() {
        let wasm = wat::parse_str(fixture_component_wat(
            &subtitle_descriptor_json(),
            "not json at all",
            "not json either",
            &host_request_bytes(),
            &expected_host_response_bytes(),
        ))
        .expect("fixture subtitle component WAT must assemble");
        let spec = test_spec(wasm, configured_command_host());

        let error = process_subtitle_component(&spec, &search_command_request(), invocation())
            .await
            .expect_err("a malformed response document must fail the invocation");

        assert!(
            error.to_string().contains("PluginCommandResponse"),
            "{error}"
        );
    }
}
