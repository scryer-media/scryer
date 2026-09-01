//! WASI Preview 2 host for notification components.
//!
//! The download-client host is this file's template, and at the level of the
//! world they are the same shape: both import the shared
//! `scryer:host/services@1.0.0` interface and both carry the command ABI's
//! [`PluginCommandRequest`]/[`PluginCommandResponse`] JSON envelope on
//! `process`. What is different about this family is not the transport but the
//! *authority* on the other side of the import.
//!
//! # This is the family that holds sockets and host processes
//!
//! An SMTP notifier does not speak HTTP. It needs a TCP stream it drives itself
//! — connect, `EHLO`, read the greeting, `STARTTLS`, keep going on the upgraded
//! stream — and a first-party script notifier needs to spawn one allowlisted
//! executable. Both arrive through the *same* `host-call` every other family
//! uses, as [`PluginHostRequest`] variants the SDK already defines, and both are
//! served by [`CommandHost`]'s socket and process arms. Nothing about that is
//! visible in this host beyond the [`CommandHost`] it is handed: a notification
//! host is one built by [`CommandHost::for_notification`], and every other
//! family's host answers those same variants with an in-band `Unsupported`.
//!
//! That is why this host does *not* link `wasi:sockets` or grow a second
//! import. Scryer's socket grant is per-channel and descriptor-shaped — host
//! pattern, port set, TLS mode, resolved against the channel's own config — and
//! a p2 socket capability is ambient within whatever network the host opens.
//! Routing the grant through the single host-call door is what keeps the check
//! on every connection.
//!
//! # Lifecycle
//!
//! Instance-per-request, exactly as the command protocol is: one `process` call
//! per plugin invocation, then the whole `Store` is dropped. The socket handle
//! table does *not* live in the store — it belongs to the channel's
//! [`CommandHost`] — so a guest that opens a socket in one `process` call could
//! in principle read it in the next. The adapter closes that gap the same way it
//! always has, by calling `SocketHost::cleanup` after every notification, so a
//! component channel leaks no more sockets between sends than a reactor one.
//!
//! This family grants no filesystem preopens on any operation, matching both
//! older runtimes.

use std::sync::Arc;
use std::time::{Duration, Instant};

use scryer_application::{AppError, AppResult};
use scryer_plugin_sdk::PluginDescriptor;
use scryer_plugin_sdk::command::{PluginCommandRequest, PluginCommandResponse};
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

mod contract_v1_0 {
    wasmtime::component::bindgen!({
        world: "scryer:notification/notification@1.0.0",
        // Two packages, two paths, host package first — the layout the subtitle
        // and download-client worlds established so
        // `import scryer:host/services@1.0.0` resolves against one canonical
        // copy of `scryer:host` with no `deps/` duplicates and no symlinks to
        // keep in sync.
        path: ["wit/host-v1.0.0", "wit/notification-v1.0.0"],
        // The WIT signature stays synchronous so a guest needs no async runtime
        // to call it, while the host implementation may await: the service
        // layer reaches HTTP, archive extraction, blocking socket I/O and
        // process execution.
        imports: { default: async },
        exports: { default: async },
    });
}

use self::contract_v1_0::InvocationError;
use self::contract_v1_0::scryer::host::services::{Host as ServicesHost, HostError};

/// Identifying context for one notification component invocation.
pub(crate) struct NotificationComponentInvocation<'a> {
    pub(crate) plugin_id: &'a str,
    pub(crate) plugin_version: &'a str,
    pub(crate) operation: &'a str,
}

/// Compile-and-link validation for a notification component artifact, mirroring
/// `validate_download_client_component`.
pub(crate) fn validate_notification_component(wasm: &[u8]) -> Result<(), String> {
    NotificationComponentRuntime::new(engine::shared_async_engine(), wasm).map(|_| ())
}

/// Store data for one notification component invocation.
pub(crate) struct NotificationComponentCtx {
    table: ResourceTable,
    wasi: WasiCtx,
    limits: HostLimits,
    command_host: CommandHost,
}

impl WasiView for NotificationComponentCtx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl ServicesHost for NotificationComponentCtx {
    /// The shared host import, backed by the channel's own [`CommandHost`].
    ///
    /// The service layer is synchronous and blocks for real on this family — a
    /// socket read waits on the wire and a `ProcessExec` waits on a child — so
    /// it runs on the blocking pool rather than on this async worker, exactly as
    /// the core-module `scryer_host_call` shim does. A `CommandHost` is a cheap
    /// `Arc` clone, so the move costs nothing and the socket handle table stays
    /// shared with the legacy registrations.
    async fn host_call(&mut self, request: Vec<u8>) -> Result<Vec<u8>, HostError> {
        match family_component_host::dispatch_host_call(&self.command_host, request).await {
            Ok(response) => Ok(response),
            Err(HostCallError::Service { failure, error }) => {
                tracing::debug!(
                    target: "scryer_plugins::notification",
                    error = error.as_str(),
                    "notification component host-call failed",
                );
                Err(match failure {
                    HostCallFailure::InvalidRequest => HostError::InvalidRequest,
                    HostCallFailure::Failed => HostError::Failed,
                })
            }
            Err(HostCallError::Task(error)) => {
                tracing::debug!(
                    target: "scryer_plugins::notification",
                    error = %error,
                    "notification component host-call task failed",
                );
                Err(HostError::Failed)
            }
        }
    }
}

/// A compiled notification component plus its pre-instantiated world binding.
pub(crate) struct NotificationComponentRuntime {
    component: Arc<Component>,
    instance_pre: contract_v1_0::NotificationPre<NotificationComponentCtx>,
}

impl NotificationComponentRuntime {
    pub(crate) fn new(engine: &Engine, wasm: &[u8]) -> Result<Self, String> {
        let component = module_cache::notification_component(wasm)?;
        if !Engine::same(component.engine(), engine) {
            return Err(
                "notification component cache returned an artifact for a different engine".into(),
            );
        }
        let mut linker = Linker::new(engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|error| format!("failed to register WASI Preview 2: {error}"))?;
        contract_v1_0::Notification::add_to_linker::<
            NotificationComponentCtx,
            HasSelf<NotificationComponentCtx>,
        >(&mut linker, |ctx| ctx)
        .map_err(|error| format!("failed to register notification component host: {error:#}"))?;
        let raw_instance_pre = linker.instantiate_pre(&component).map_err(|error| {
            format!("failed to preinstantiate notification component: {error:#}")
        })?;
        let instance_pre = contract_v1_0::NotificationPre::new(raw_instance_pre).map_err(|error| {
            format!(
                "notification component exports do not match scryer:notification/notification@1.0.0: {error:#}"
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
    ) -> Result<(Store<NotificationComponentCtx>, contract_v1_0::Notification), wasmtime::Error>
    {
        let mut store = Store::new(
            self.component.engine(),
            NotificationComponentCtx {
                table: ResourceTable::new(),
                wasi,
                limits: HostLimits::new(memory_max_bytes),
                command_host,
            },
        );
        store.limiter(|ctx: &mut NotificationComponentCtx| &mut ctx.limits);
        store.set_epoch_deadline(engine::deadline_ticks(timeout));
        let plugin = self.instance_pre.instantiate_async(&mut store).await?;
        Ok((store, plugin))
    }
}

/// Extract a descriptor from a notification component through the world's
/// `describe` export.
///
/// The loader's descriptor path is synchronous while component guests run on
/// the async engine, so the call is driven on a private current-thread runtime
/// on its own thread — safe from inside a Tokio worker (no nested `block_on`)
/// and from a plain thread alike. Describe happens on install and reload, never
/// per invocation.
pub(crate) fn notification_component_describe(wasm: &[u8]) -> Result<PluginDescriptor, String> {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| {
                        format!("failed to start notification component describe runtime: {error}")
                    })?;
                runtime.block_on(describe_async(wasm))
            })
            .join()
            .map_err(|_| "notification component describe thread panicked".to_string())?
    })
}

async fn describe_async(wasm: &[u8]) -> Result<PluginDescriptor, String> {
    let runtime = NotificationComponentRuntime::new(engine::shared_async_engine(), wasm)?;
    let (wasi, stderr) = sandbox::build_component_describe_sandbox();
    // Describe is a pure function of the artifact: no services, and in
    // particular no sockets and no process execution, so a guest that reaches
    // for the host during describe is told `Unsupported` in-band rather than
    // being handed a configured channel's authority.
    let (mut store, plugin) = runtime
        .instantiate(wasi, CommandHost::disabled(), None, DESCRIBE_TIMEOUT)
        .await
        .map_err(|error| {
            format!("failed to instantiate notification component for describe: {error:#}")
        })?;
    let descriptor_json = plugin.call_describe(&mut store).await.map_err(|error| {
        let denied = store.data().limits.memory_denied;
        let failure = error::classify_error(&error, denied);
        let stderr_tail = tail_of(&stderr);
        format!(
            "notification component describe failed ({:?}): {}{}",
            failure.kind,
            failure.detail,
            stderr_suffix(&stderr_tail)
        )
    })?;
    serde_json::from_slice::<PluginDescriptor>(&descriptor_json).map_err(|error| {
        format!("notification component describe returned invalid PluginDescriptor JSON: {error}")
    })
}

/// Compile (or reuse) the component off the async worker, with the same
/// preparation timeout the other component paths use.
async fn prepare_notification_component(
    wasm: Arc<Vec<u8>>,
    timeout: Duration,
) -> Result<NotificationComponentRuntime, String> {
    let prepare = tokio::task::spawn_blocking(move || {
        NotificationComponentRuntime::new(engine::shared_async_engine(), &wasm)
    });
    match tokio::time::timeout(timeout, prepare).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(format!(
            "notification component preparation task failed: {error}"
        )),
        Err(_) => Err(format!(
            "timed out waiting for notification component rehydration after {} ms",
            timeout.as_millis()
        )),
    }
}

/// Instantiate the notification component and run one command
/// request→response exchange.
pub(crate) async fn process_notification_component(
    spec: &PluginInstanceSpec,
    request: &PluginCommandRequest,
    invocation: NotificationComponentInvocation<'_>,
) -> AppResult<PluginCommandResponse> {
    let span = tracing::info_span!(
        "notification_plugin_invoke",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
    );
    let _enter = span.enter();

    let started = Instant::now();
    let request_bytes = serde_json::to_vec(request).map_err(|error| {
        AppError::Repository(format!(
            "failed to serialize notification plugin command: {error}"
        ))
    })?;
    let request_len = request_bytes.len();

    let runtime = prepare_notification_component(Arc::clone(&spec.wasm), spec.timeout)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "notification plugin {}@{} failed to prepare: {error}",
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
            target: "scryer_plugins::notification",
            plugin_id = invocation.plugin_id,
            stderr = stderr_tail.as_str(),
            "notification plugin stderr",
        );
    }

    let response_bytes = match call_result {
        Ok(Ok(response_bytes)) => response_bytes,
        Ok(Err(invocation_error)) => {
            let failure = error::protocol_failure(format!(
                "notification component reported {}",
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
                "notification component returned invalid PluginCommandResponse JSON: {error}"
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
            "notification component response used unsupported ABI version {}",
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
        target: "scryer_plugins::notification",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
        duration_ms = started.elapsed().as_millis() as u64,
        request_bytes = request_len,
        response_bytes = response_bytes.len(),
        disposition = "ok",
        "notification plugin invocation complete",
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
    invocation: &NotificationComponentInvocation<'_>,
    budget: Duration,
    stderr_tail: &str,
    failure: &error::RunFailure,
    started: Instant,
    request_len: usize,
) -> AppError {
    tracing::debug!(
        target: "scryer_plugins::notification",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
        duration_ms = started.elapsed().as_millis() as u64,
        request_bytes = request_len,
        disposition = ?failure.kind,
        "notification plugin invocation failed",
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
pub(crate) mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use scryer_plugin_sdk::PluginResult;
    use scryer_plugin_sdk::command::{
        PluginCommand, PluginCommandResult, PluginNotificationCommand,
        PluginNotificationCommandResult,
    };
    use scryer_plugin_sdk::host::{PluginHostRequest, PluginHostResponse};
    use scryer_plugin_sdk::{
        PluginNotificationResponse, SocketCloseRequest, SocketOpenRequest, SocketPermission,
        SocketReadRequest, SocketReadResponse, SocketTlsMode, SocketWriteRequest,
    };

    use crate::process_host::ProcessHost;
    use crate::socket_host::SocketHost;

    /// Guest memory layout for the hand-built fixture component below.
    const DESCRIPTOR_PTR: usize = 0;
    const OK_RESPONSE_PTR: usize = 8192;
    const FAIL_RESPONSE_PTR: usize = 12288;
    /// Requests fired and ignored before the compared one, 1 KiB apart.
    const PRE_CALL_BASE: usize = 16384;
    const PRE_CALL_STRIDE: usize = 1024;
    const FINAL_REQUEST_PTR: usize = 24576;
    const EXPECTED_RESPONSE_PTR: usize = 26624;
    const DESCRIBE_RETURN_PTR: usize = 30720;
    const PROCESS_RETURN_PTR: usize = 30736;
    const HOST_RETURN_PTR: usize = 30752;

    /// What the fixture guest writes at the loopback listener, and what the
    /// listener answers with. Both are small enough to cross in one segment.
    const SOCKET_PROBE: &[u8] = b"EHLO scryer\r\n";
    pub(crate) const SOCKET_REPLY: &[u8] = b"250 OK\r\n";

    /// WAT data-string escaping: every byte as `\xx`.
    fn wat_bytes(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("\\{byte:02x}")).collect()
    }

    fn encode_request(request: &PluginHostRequest) -> Vec<u8> {
        postcard::to_allocvec(request).expect("fixture host request must encode")
    }

    pub(crate) fn notification_descriptor(
        socket_permissions: Vec<SocketPermission>,
    ) -> PluginDescriptor {
        PluginDescriptor {
            id: "fixture-notification".to_string(),
            name: "Fixture Notification".to_string(),
            version: "1.0.0".to_string(),
            sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
            sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
            socket_permissions,
            provider: scryer_plugin_sdk::ProviderDescriptor::Notification(
                scryer_plugin_sdk::NotificationDescriptor {
                    provider_type: "fixture-notification".to_string(),
                    provider_aliases: Vec::new(),
                    config_fields: Vec::new(),
                    default_base_url: None,
                    allowed_hosts: Vec::new(),
                    capabilities: scryer_plugin_sdk::NotificationCapabilities::default(),
                },
            ),
        }
    }

    fn notification_descriptor_json(descriptor: &PluginDescriptor) -> String {
        serde_json::to_string(descriptor).expect("fixture descriptor must serialize")
    }

    /// The document the guest returns once the socket round trip verified.
    fn ok_response_json() -> String {
        serde_json::to_string(&PluginCommandResponse::new(
            PluginCommandResult::Notification(PluginNotificationCommandResult::Send(
                PluginResult::Ok(PluginNotificationResponse {
                    success: true,
                    // The evidence: the guest names the bytes it could only
                    // have read off a real socket the host opened for it.
                    delivery_id: Some(String::from_utf8_lossy(SOCKET_REPLY).trim().to_string()),
                    error: None,
                    provider_status: None,
                    retry_after_seconds: None,
                    warnings: Vec::new(),
                    target_results: Vec::new(),
                }),
            )),
        ))
        .expect("fixture ok response must serialize")
    }

    /// The document the guest returns when the round trip did NOT verify.
    fn fail_response_json() -> String {
        serde_json::to_string(&PluginCommandResponse::new(
            PluginCommandResult::Notification(PluginNotificationCommandResult::Send(
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

    /// A minimal but real `scryer:notification/notification@1.0.0` component.
    ///
    /// `describe` returns a static descriptor document. `process` fires
    /// `pre_calls` through `scryer:host/services@1.0.0` and ignores their
    /// payloads, then issues `final_request` and compares the response
    /// byte-for-byte with `expected_response`. Wiring the fixture that way lets
    /// one WAT body express the whole socket sequence — open, write, read,
    /// with the *read* being what the assertion rides on — because only a
    /// correctly opened and written socket can produce the expected read bytes.
    fn fixture_component_wat(
        descriptor_json: &str,
        ok_json: &str,
        fail_json: &str,
        pre_calls: &[Vec<u8>],
        final_request: &[u8],
        expected_response: &[u8],
    ) -> String {
        let pre_call_data = pre_calls
            .iter()
            .enumerate()
            .map(|(index, request)| {
                format!(
                    "    (data (i32.const {ptr}) \"{bytes}\")\n",
                    ptr = PRE_CALL_BASE + index * PRE_CALL_STRIDE,
                    bytes = wat_bytes(request),
                )
            })
            .collect::<String>();
        let pre_call_body = pre_calls
            .iter()
            .enumerate()
            .map(|(index, request)| {
                format!(
                    r#"      (call $host_call
        (i32.const {ptr}) (i32.const {len}) (i32.const {host_ret}))
      (if (i32.ne (i32.load8_u (i32.const {host_ret})) (i32.const 0))
        (then (return (call $fail))))
"#,
                    ptr = PRE_CALL_BASE + index * PRE_CALL_STRIDE,
                    len = request.len(),
                    host_ret = HOST_RETURN_PTR,
                )
            })
            .collect::<String>();

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
{pre_call_data}    (data (i32.const {final_ptr}) "{final_req}")
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
{pre_call_body}      ;; The call the assertion rides on.
      (call $host_call
        (i32.const {final_ptr}) (i32.const {final_len}) (i32.const {host_ret}))
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
            pre_call_data = pre_call_data,
            pre_call_body = pre_call_body,
            final_ptr = FINAL_REQUEST_PTR,
            final_req = wat_bytes(final_request),
            final_len = final_request.len(),
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

    /// The socket sequence the fixture drives: open the loopback listener,
    /// write a probe, then read the reply. `port` is the listener's, so the
    /// grant under test is the descriptor's, resolved against a real address.
    fn socket_pre_calls(port: u16) -> Vec<Vec<u8>> {
        vec![
            encode_request(&PluginHostRequest::SocketOpen(SocketOpenRequest {
                host: "127.0.0.1".to_string(),
                port,
                tls_mode: SocketTlsMode::Plain,
                connect_timeout_ms: Some(5_000),
                read_timeout_ms: Some(5_000),
                write_timeout_ms: Some(5_000),
            })),
            encode_request(&PluginHostRequest::SocketWrite(SocketWriteRequest {
                // The first socket a fresh host opens is always handle 1.
                handle: 1,
                data_base64: STANDARD.encode(SOCKET_PROBE),
            })),
        ]
    }

    fn socket_read_request() -> Vec<u8> {
        encode_request(&PluginHostRequest::SocketRead(SocketReadRequest {
            handle: 1,
            max_bytes: 64,
        }))
    }

    fn expected_socket_read_response() -> Vec<u8> {
        postcard::to_allocvec(&PluginHostResponse::SocketRead(PluginResult::Ok(
            SocketReadResponse {
                data_base64: STANDARD.encode(SOCKET_REPLY),
                eof: false,
            },
        )))
        .expect("fixture socket-read response must encode")
    }

    /// A loopback listener that answers one connection with [`SOCKET_REPLY`].
    ///
    /// Returned with its port so the descriptor grant and the guest's
    /// `SocketOpen` name the same address.
    pub(crate) fn echo_listener() -> (u16, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener must bind");
        let port = listener
            .local_addr()
            .expect("listener must report its address")
            .port();
        let handle = std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut probe = vec![0_u8; SOCKET_PROBE.len()];
            let _ = stream.read_exact(&mut probe);
            let _ = stream.write_all(SOCKET_REPLY);
            let _ = stream.flush();
            // Hold the connection open until the guest has read; dropping the
            // listener side early would race the read with a FIN.
            std::thread::sleep(Duration::from_millis(250));
        });
        (port, handle)
    }

    pub(crate) fn loopback_permission(port: u16) -> SocketPermission {
        SocketPermission {
            host_pattern: "127.0.0.1".to_string(),
            ports: vec![port],
            tls_modes: vec![SocketTlsMode::Plain],
        }
    }

    /// A fixture whose whole `process` answer is gated on a real socket round
    /// trip through the shared host-call door.
    pub(crate) fn socket_fixture_component(descriptor: &PluginDescriptor, port: u16) -> Vec<u8> {
        wat::parse_str(fixture_component_wat(
            &notification_descriptor_json(descriptor),
            &ok_response_json(),
            &fail_response_json(),
            &socket_pre_calls(port),
            &socket_read_request(),
            &expected_socket_read_response(),
        ))
        .expect("fixture notification component WAT must assemble")
    }

    /// The descriptor-only fixture used by the detection and describe tests,
    /// where no host is configured and the socket sequence is beside the point.
    pub(crate) fn fixture_component() -> Vec<u8> {
        socket_fixture_component(&notification_descriptor(Vec::new()), 1)
    }

    fn command_host_for(descriptor: &PluginDescriptor) -> CommandHost {
        CommandHost::for_notification(
            descriptor.id.clone(),
            BTreeMap::new(),
            Vec::new(),
            Duration::from_secs(30),
            None,
            None,
            SocketHost::from_descriptor(descriptor, None),
            ProcessHost::disabled(),
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

    /// A minimal well-formed `PluginNotificationRequest`.
    ///
    /// The fixture only checks that the envelope crossed as JSON, so the
    /// document's content is beside the point; building it through serde rather
    /// than by hand keeps the test from restating twenty optional fields.
    pub(crate) fn send_request() -> PluginCommandRequest {
        let request = serde_json::json!({
            "event_type": serde_json::to_value(
                scryer_plugin_sdk::NotificationEventType::Test,
            )
            .expect("the event type serializes"),
            "summary_title": "fixture",
            "summary_message": "fixture",
            "app": { "name": "scryer", "version": "0.0.0" },
        });
        PluginCommandRequest::new(PluginCommand::Notification(
            PluginNotificationCommand::Send(
                serde_json::from_value(request).expect("the fixture notification request decodes"),
            ),
        ))
    }

    fn invocation() -> NotificationComponentInvocation<'static> {
        NotificationComponentInvocation {
            plugin_id: "fixture-notification",
            plugin_version: "1.0.0",
            operation: "notification_send",
        }
    }

    fn send_result(response: PluginCommandResponse) -> PluginResult<PluginNotificationResponse> {
        let PluginCommandResult::Notification(PluginNotificationCommandResult::Send(result)) =
            response.response
        else {
            panic!("fixture must answer a send command with a send result");
        };
        result
    }

    #[test]
    fn a_core_module_notification_artifact_fails_world_validation() {
        let core_module = wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "_start")))"#,
        )
        .expect("core module WAT must parse");

        let error = validate_notification_component(&core_module)
            .expect_err("a core module must not validate as a notification component");
        assert!(
            error.contains("component") || error.contains("compile"),
            "{error}"
        );
    }

    #[test]
    fn an_arbitrary_component_fails_world_validation() {
        let wasm = wat::parse_str("(component)").expect("component WAT must parse");

        let error = validate_notification_component(&wasm)
            .expect_err("an arbitrary component must not pass notification-world validation");
        assert!(error.contains("exports do not match"), "{error}");
    }

    #[test]
    fn the_fixture_component_passes_world_validation() {
        validate_notification_component(&fixture_component())
            .expect("the fixture must satisfy scryer:notification/notification@1.0.0");
    }

    /// The archive world exports the same two functions and is told apart only
    /// by what it imports, so a notification component must not silently link
    /// as an archive extractor.
    #[test]
    fn a_notification_component_does_not_validate_as_an_archive_component() {
        let error = crate::wasmtime_host::validate_archive_component(&fixture_component())
            .expect_err("a notification component must not satisfy the archive world");
        assert!(!error.is_empty(), "{error}");
    }

    /// The subtitle, download-client and notification worlds import the same
    /// shared services interface and export the same two functions, so at the
    /// component-type level they are the *same* world and each validates as the
    /// others. That is not a gap: family separation is the descriptor's job —
    /// the loader picks a backing from `PluginDescriptor::provider` — and this
    /// test pins the property so a future world change does not silently start
    /// relying on structural separation that was never there.
    ///
    /// It is a sharper statement for this family than for the others: a
    /// notification component and a subtitle component are structurally
    /// interchangeable, and yet only one of them is ever handed a socket,
    /// because the authority rides on the `CommandHost` the loader builds and
    /// not on the world.
    #[test]
    fn the_services_worlds_are_structurally_the_same_world() {
        let subtitle = crate::wasmtime_host::subtitle_component_host::tests::fixture_component();

        validate_notification_component(&subtitle)
            .expect("the families share a world shape; only the descriptor tells them apart");
        crate::wasmtime_host::validate_subtitle_component(&fixture_component())
            .expect("and symmetrically");
        crate::wasmtime_host::validate_download_client_component(&fixture_component())
            .expect("and against the download-client world too");
    }

    #[test]
    fn describe_returns_the_guest_descriptor() {
        let descriptor = notification_component_describe(&fixture_component())
            .expect("the fixture must self-describe through the world's describe export");

        assert_eq!(descriptor.id, "fixture-notification");
        assert!(matches!(
            descriptor.provider,
            scryer_plugin_sdk::ProviderDescriptor::Notification(_)
        ));
    }

    /// The end-to-end host path, and the point of this family's world: the
    /// command envelope crosses as JSON, and the guest opens a real TCP socket,
    /// writes to it and reads the answer back — all five socket operations
    /// riding the one shared `scryer:host/services@1.0.0` import, gated by the
    /// descriptor's own `socket_permissions`.
    #[tokio::test(flavor = "multi_thread")]
    async fn process_drives_a_real_socket_round_trip_through_the_shared_host_call() {
        let (port, listener) = echo_listener();
        let descriptor = notification_descriptor(vec![loopback_permission(port)]);
        let spec = test_spec(
            socket_fixture_component(&descriptor, port),
            command_host_for(&descriptor),
        );

        let response = process_notification_component(&spec, &send_request(), invocation())
            .await
            .expect("the fixture component must complete one process exchange");
        listener.join().ok();

        let PluginResult::Ok(response) = send_result(response) else {
            panic!("a plugin error means the socket round trip did not complete through host-call");
        };
        assert_eq!(
            response.delivery_id.as_deref(),
            Some("250 OK"),
            "the guest must have read the listener's reply back through host-call",
        );
    }

    /// The gate: the *same* guest against the *same* listener, with a
    /// descriptor that grants no socket permissions, cannot complete the round
    /// trip. This is what makes the success above a statement about authority
    /// rather than about reachability — nothing else changes between the two
    /// tests.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_descriptor_without_socket_permissions_cannot_open_the_socket() {
        let (port, listener) = echo_listener();
        let granted = notification_descriptor(vec![loopback_permission(port)]);
        let ungranted = notification_descriptor(Vec::new());
        let spec = test_spec(
            socket_fixture_component(&granted, port),
            command_host_for(&ungranted),
        );

        let response = process_notification_component(&spec, &send_request(), invocation())
            .await
            .expect("a denied socket must not fail the invocation itself");
        drop(listener);

        assert!(
            matches!(send_result(response), PluginResult::Err(_)),
            "a denied socket must not look like a completed round trip",
        );
    }

    /// A guest whose expectation of the host response is wrong takes its
    /// failure branch — proof that the success above is the round trip and not
    /// a guest that ignores the host.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_mismatched_host_response_drives_the_guest_failure_branch() {
        let (port, listener) = echo_listener();
        let descriptor = notification_descriptor(vec![loopback_permission(port)]);
        let mut expected = expected_socket_read_response();
        expected.push(0xff);
        let wasm = wat::parse_str(fixture_component_wat(
            &notification_descriptor_json(&descriptor),
            &ok_response_json(),
            &fail_response_json(),
            &socket_pre_calls(port),
            &socket_read_request(),
            &expected,
        ))
        .expect("fixture notification component WAT must assemble");
        let spec = test_spec(wasm, command_host_for(&descriptor));

        let response = process_notification_component(&spec, &send_request(), invocation())
            .await
            .expect("the fixture component must still complete the exchange");
        drop(listener);

        let PluginResult::Err(error) = send_result(response) else {
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
        let (port, listener) = echo_listener();
        let descriptor = notification_descriptor(vec![loopback_permission(port)]);
        let spec = test_spec(
            socket_fixture_component(&descriptor, port),
            CommandHost::disabled(),
        );

        let response = process_notification_component(&spec, &send_request(), invocation())
            .await
            .expect("a disabled host must not fail the invocation itself");
        drop(listener);

        assert!(
            matches!(send_result(response), PluginResult::Err(_)),
            "an unconfigured service must not look like a completed round trip",
        );
    }

    /// A notification host built without the socket service — what every other
    /// family's `CommandHost` is — refuses the same sequence, in-band.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_host_without_the_socket_service_reports_unsupported_in_band() {
        let (port, listener) = echo_listener();
        let descriptor = notification_descriptor(vec![loopback_permission(port)]);
        let spec = test_spec(
            socket_fixture_component(&descriptor, port),
            CommandHost::with_archive_provider(
                descriptor.id.clone(),
                BTreeMap::new(),
                Vec::new(),
                Duration::from_secs(30),
                None,
                None,
            ),
        );

        let response = process_notification_component(&spec, &send_request(), invocation())
            .await
            .expect("an absent socket service must not fail the invocation itself");
        drop(listener);

        assert!(
            matches!(send_result(response), PluginResult::Err(_)),
            "a host with no socket service must not complete a socket round trip",
        );
    }

    /// A `process` response that is not a `PluginCommandResponse` is a protocol
    /// failure, not a silent empty result.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_non_json_process_response_is_a_protocol_failure() {
        let descriptor = notification_descriptor(Vec::new());
        let wasm = wat::parse_str(fixture_component_wat(
            &notification_descriptor_json(&descriptor),
            "not json at all",
            "not json either",
            &[encode_request(&PluginHostRequest::SocketClose(
                SocketCloseRequest { handle: 1 },
            ))],
            &socket_read_request(),
            &expected_socket_read_response(),
        ))
        .expect("fixture notification component WAT must assemble");
        let spec = test_spec(wasm, command_host_for(&descriptor));

        let error = process_notification_component(&spec, &send_request(), invocation())
            .await
            .expect_err("a malformed response document must fail the invocation");

        assert!(
            error.to_string().contains("PluginCommandResponse"),
            "{error}"
        );
    }
}
