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
//!
//! # Two contracts on one linker
//!
//! This host serves both `scryer:subtitle/subtitle-provider@1.0.0` and `@1.1.0`,
//! the same way the indexer host serves its own 1.0 and 1.1. The revision
//! changes two things: `process` is lifted into the canonical ABI's async form,
//! and the world imports `scryer:runtime/host@1.0.0` alongside the encoded
//! door. Both worlds are registered on one `Linker` and the artifact decides
//! which it is — 1.1 is tried first, and 1.0 is the fallback — so an installed
//! 1.0 provider keeps running untouched across the upgrade.
//!
//! The typed runtime import does **not** get a second implementation of
//! anything. `http` builds the SDK's `PluginHostRequest::Http` and goes through
//! [`family_component_host::dispatch_host_call`] into the very same
//! [`CommandHost`] the encoded door uses, so allowed-host enforcement, the
//! response cap, the proxy policy and the per-invocation deadline are enforced
//! once, in one place; the typed binding only projects the service layer's
//! `PluginError` onto the world's `transport-error` (see
//! [`http_transport_error`] for that table). `config-get` and `state-get` read
//! the same maps the `ConfigGet`/`StateGet` arms read, and `state-cas` is a
//! `CommandHost` method that takes that same lock once rather than a
//! get-then-set the guest could race against itself.

use std::sync::Arc;
use std::time::{Duration, Instant};

use scryer_application::{AppError, AppResult};
use scryer_plugin_sdk::command::{PluginCommandRequest, PluginCommandResponse};
use scryer_plugin_sdk::host::{
    PluginHostRequest, PluginHostResponse, PluginHttpRequest as SdkPluginHttpRequest,
};
use scryer_plugin_sdk::{PluginDescriptor, PluginError, PluginErrorCode, PluginResult};
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

mod contract_v1_1 {
    wasmtime::component::bindgen!({
        world: "scryer:subtitle/subtitle-provider@1.1.0",
        // Three packages, three paths: the shared encoded door, the typed
        // family-neutral runtime, and the family world itself. Same layout
        // rule as 1.0 — one canonical copy of each package, no `deps/`
        // duplicates and no symlinks to keep in sync.
        path: ["wit/host-v1.0.0", "wit/runtime-v1.0.0", "wit/subtitle-v1.1.0"],
        // `host-call`, `config-get`, `state-cas` and the rest stay synchronous
        // in WIT so a guest needs no async runtime to call them, while the host
        // implementations may await — the service layer reaches HTTP and
        // archive extraction. `http` and `sleep` are `async func` in WIT and
        // are therefore bound through `HostWithStore` instead of this trait.
        imports: { default: async },
        // `describe` is a plain sync export and `process` is an `async func`;
        // this makes the former callable on the async store the shared engine
        // gives us, and leaves the latter driven through `run_concurrent`.
        exports: { default: async },
    });
}

use self::contract_v1_0::InvocationError;
use self::contract_v1_0::scryer::host::services::{Host as ServicesHost, HostError};

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
    /// The zero of the monotonic clock this instance reports to the guest.
    ///
    /// The guest only ever sees differences, so the origin can be per-instance;
    /// what matters is that `monotonic-now-ms` and
    /// `operation-deadline-monotonic-ms` share it, which is what lets a plugin
    /// budget its own pacing against the host's deadline.
    clock_origin: Instant,
    /// When this invocation must be finished, on the same timebase.
    deadline: Instant,
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
        match family_component_host::dispatch_host_call(&self.command_host, request).await {
            Ok(response) => Ok(response),
            Err(HostCallError::Service { failure, error }) => {
                tracing::debug!(
                    target: "scryer_plugins::subtitle",
                    error = error.as_str(),
                    "subtitle component host-call failed",
                );
                Err(match failure {
                    HostCallFailure::InvalidRequest => HostError::InvalidRequest,
                    HostCallFailure::Failed => HostError::Failed,
                })
            }
            Err(HostCallError::Task(error)) => {
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

/// The typed, family-neutral runtime import of contract 1.1.
///
/// Everything here answers from the same [`CommandHost`] the encoded door
/// answers from — the same config map, the same state map under the same lock
/// — so a 1.1 guest and a 1.0 guest observe one host, not two. What the typed
/// binding buys is shape: an `option<string>` instead of a postcard round trip
/// for a `BTreeMap` lookup a pacing loop makes constantly, and a real
/// compare-and-swap instead of a get-then-set.
impl contract_v1_1::scryer::runtime::host::Host for SubtitleComponentCtx {
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

    /// Subtitle instances carry no provider profile.
    ///
    /// [`PluginInstanceSpec`] has no field for one — profiles are an indexer
    /// concept, bound to a provider's protocol dialect — so this is `none` for
    /// every subtitle provider rather than an empty document that a guest would
    /// have to tell apart from a real one. Shared engine code must treat `none`
    /// as "use your defaults", which is what `newznab-common` already does.
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

    async fn log(
        &mut self,
        level: contract_v1_1::scryer::runtime::host::LogLevel,
        message: String,
    ) {
        use contract_v1_1::scryer::runtime::host::LogLevel;

        match level {
            LogLevel::Trace => tracing::trace!(target: "scryer_plugins::subtitle", "{message}"),
            LogLevel::Debug => tracing::debug!(target: "scryer_plugins::subtitle", "{message}"),
            LogLevel::Info => tracing::info!(target: "scryer_plugins::subtitle", "{message}"),
            LogLevel::Warn => tracing::warn!(target: "scryer_plugins::subtitle", "{message}"),
            LogLevel::Error => tracing::error!(target: "scryer_plugins::subtitle", "{message}"),
        }
    }
}

/// The concurrent half of the runtime import: the two `async func`s.
impl contract_v1_1::scryer::runtime::host::HostWithStore<SubtitleComponentCtx>
    for HasSelf<SubtitleComponentCtx>
{
    /// One HTTP request, through the *same* service layer the encoded door
    /// uses.
    ///
    /// This is the rule the whole revision hangs on: there is no second HTTP
    /// implementation for subtitle providers. The typed request is re-encoded
    /// as the SDK's `PluginHostRequest::Http` and handed to
    /// [`family_component_host::dispatch_host_call`], so allowed-host
    /// enforcement, the response-size cap, the assigned proxy policy and the
    /// remaining per-invocation budget are applied once, by [`CommandHost`],
    /// exactly as they are for a 1.0 guest calling `host-call` directly. The
    /// only thing that happens here is the projection back onto the world's
    /// types.
    async fn http(
        accessor: &wasmtime::component::Accessor<SubtitleComponentCtx, Self>,
        request: contract_v1_1::scryer::runtime::host::HttpRequest,
    ) -> Result<
        contract_v1_1::scryer::runtime::host::HttpResponse,
        contract_v1_1::scryer::runtime::host::TransportError,
    > {
        use contract_v1_1::scryer::runtime::host::{Header, HttpResponse, TransportError};

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
                        target: "scryer_plugins::subtitle",
                        error = error.as_str(),
                        "subtitle component typed http request was rejected by the host",
                    );
                    return Err(TransportError::InvalidRequest);
                }
                Err(HostCallError::Service { error, .. } | HostCallError::Task(error)) => {
                    tracing::debug!(
                        target: "scryer_plugins::subtitle",
                        error = error.as_str(),
                        "subtitle component typed http request failed in transport",
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
    ///
    /// Cancellation is structural rather than a token: the guest's `process`
    /// future — and this future with it — is dropped when the host abandons the
    /// invocation, and the epoch deadline still interrupts a guest that never
    /// yields. The deadline cap is what stops a plugin from parking past its
    /// own budget and turning a pacing decision into a timeout.
    async fn sleep(
        accessor: &wasmtime::component::Accessor<SubtitleComponentCtx, Self>,
        duration_ms: u64,
    ) {
        let deadline = accessor.with(|mut access| access.get().deadline);
        let remaining = deadline.saturating_duration_since(Instant::now());
        tokio::time::sleep(Duration::from_millis(duration_ms).min(remaining)).await;
    }
}

/// Project the service layer's `PluginError` onto the world's
/// `transport-error`.
///
/// The encoded door reports HTTP failures as a `PluginError` whose detail lives
/// in `debug_message`, because `CommandHost` composes messages from
/// `PluginHttpHost`'s `Result<_, String>`. The typed world wants one of seven
/// enum cases, so this is the projection, and it is deliberately written as a
/// table rather than a chain of guesses:
///
/// | service-layer outcome                                     | `transport-error`    |
/// |-----------------------------------------------------------|----------------------|
/// | `PluginErrorCode::Unsupported` — no HTTP service here      | `forbidden-origin`   |
/// | `HTTP request to … is not allowed` — allowed-hosts denial  | `forbidden-origin`   |
/// | `Invalid URL …` / `Invalid URL scheme: …`                  | `invalid-request`    |
/// | `Proxy … is disabled.` / `proxy only supports GET …`       | `invalid-request`    |
/// | `… timed out after … ms` / `HTTP deadline exhausted`       | `timeout`            |
/// | `HTTP response exceeds the configured maximum …`           | `response-too-large` |
/// | anything else (worker stopped, DNS, TLS, socket)           | `transport`          |
///
/// Two cases the world declares are unreachable on this path, and that is
/// intentional rather than an omission. `cancelled` never appears because
/// cancellation here is structural — the future is dropped, so nothing returns
/// — and `capacity` never appears because the encoded door has no in-flight
/// governor of its own; the indexer host's per-actor and per-process HTTP
/// counters live on `ComponentHost`, which subtitle providers do not use. A
/// guest must therefore not treat either as a signal it will observe.
fn http_transport_error(
    error: &PluginError,
) -> contract_v1_1::scryer::runtime::host::TransportError {
    use contract_v1_1::scryer::runtime::host::TransportError;

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

/// Which revision of the subtitle world an artifact implements.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SubtitleContractVersion {
    V1_0,
    V1_1,
}

impl SubtitleContractVersion {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::V1_0 => "scryer:subtitle/subtitle-provider@1.0.0",
            Self::V1_1 => "scryer:subtitle/subtitle-provider@1.1.0",
        }
    }
}

enum SubtitleInstancePre {
    V1_0(contract_v1_0::SubtitleProviderPre<SubtitleComponentCtx>),
    V1_1(contract_v1_1::SubtitleProviderPre<SubtitleComponentCtx>),
}

enum SubtitlePlugin {
    V1_0(contract_v1_0::SubtitleProvider),
    V1_1(contract_v1_1::SubtitleProvider),
}

/// The import that separates the two subtitle contracts.
///
/// `scryer:subtitle@1.0.0` imports `scryer:host/services@1.0.0` and nothing
/// else; `@1.1.0` adds the typed async capability surface. A guest that names
/// this interface is speaking 1.1 whatever its export signatures look like.
const RUNTIME_HOST_IMPORT: &str = "scryer:runtime/host@1.0.0";

/// Whether `component` imports [`RUNTIME_HOST_IMPORT`].
fn imports_runtime_host(engine: &Engine, component: &Component) -> bool {
    component
        .component_type()
        .imports(engine)
        .any(|(name, _)| name == RUNTIME_HOST_IMPORT)
}

/// A compiled subtitle component plus its pre-instantiated world binding.
pub(crate) struct SubtitleComponentRuntime {
    component: Arc<Component>,
    instance_pre: SubtitleInstancePre,
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
        // `scryer:host/services@1.0.0` is the *same* interface in both
        // contracts, so it is registered once — through the 1.0 world, which is
        // the only import that world has. Registering the 1.1 world wholesale
        // would try to define that interface a second time and fail. What 1.1
        // adds is `scryer:runtime/host@1.0.0`, so only that interface is added
        // on top; a 1.0 artifact simply never imports it.
        contract_v1_0::SubtitleProvider::add_to_linker::<
            SubtitleComponentCtx,
            HasSelf<SubtitleComponentCtx>,
        >(&mut linker, |ctx| ctx)
        .map_err(|error| format!("failed to register subtitle component host: {error:#}"))?;
        contract_v1_1::scryer::runtime::host::add_to_linker::<
            SubtitleComponentCtx,
            HasSelf<SubtitleComponentCtx>,
        >(&mut linker, |ctx| ctx)
        .map_err(|error| format!("failed to register subtitle runtime host: {error:#}"))?;
        // Which contract an artifact speaks is decided by what it *imports*, not
        // by the shape of its exports. wasmtime's export type-check is lenient
        // in both directions here — a sync-lifted `process` satisfies the 1.1
        // world and an async-lifted one satisfies the 1.0 world — so the export
        // signature cannot tell the two revisions apart. The runtime import
        // can: only a 1.1 guest names `scryer:runtime/host@1.0.0`.
        let declared = if imports_runtime_host(engine, &component) {
            SubtitleContractVersion::V1_1
        } else {
            SubtitleContractVersion::V1_0
        };
        let instance_pre = Self::bind(&linker, &component, declared)?;
        Ok(Self {
            component,
            instance_pre,
        })
    }

    /// Bind `component` to `declared`, falling back to the other revision so a
    /// mislabelled artifact still loads if it genuinely fits. Both errors are
    /// carried into the message: a malformed artifact must not read as merely
    /// "not 1.1".
    fn bind(
        linker: &Linker<SubtitleComponentCtx>,
        component: &Component,
        declared: SubtitleContractVersion,
    ) -> Result<SubtitleInstancePre, String> {
        let preinstantiate = || {
            linker
                .instantiate_pre(component)
                .map_err(|error| format!("failed to preinstantiate subtitle component: {error:#}"))
        };
        let first = preinstantiate()?;
        let first_error = match declared {
            SubtitleContractVersion::V1_1 => match contract_v1_1::SubtitleProviderPre::new(first) {
                Ok(bound) => return Ok(SubtitleInstancePre::V1_1(bound)),
                Err(error) => error,
            },
            SubtitleContractVersion::V1_0 => match contract_v1_0::SubtitleProviderPre::new(first) {
                Ok(bound) => return Ok(SubtitleInstancePre::V1_0(bound)),
                Err(error) => error,
            },
        };
        let second = preinstantiate()?;
        let second_error = match declared {
            SubtitleContractVersion::V1_1 => {
                match contract_v1_0::SubtitleProviderPre::new(second) {
                    Ok(bound) => return Ok(SubtitleInstancePre::V1_0(bound)),
                    Err(error) => error,
                }
            }
            SubtitleContractVersion::V1_0 => {
                match contract_v1_1::SubtitleProviderPre::new(second) {
                    Ok(bound) => return Ok(SubtitleInstancePre::V1_1(bound)),
                    Err(error) => error,
                }
            }
        };
        let (v1_1_error, v1_0_error) = match declared {
            SubtitleContractVersion::V1_1 => (first_error, second_error),
            SubtitleContractVersion::V1_0 => (second_error, first_error),
        };
        Err(format!(
            "subtitle component exports are incompatible with scryer:subtitle/subtitle-provider@1.1.0 ({v1_1_error:#}) and @1.0.0 ({v1_0_error:#})"
        ))
    }

    /// Which revision of the world this artifact was bound to.
    pub(crate) const fn contract_version(&self) -> SubtitleContractVersion {
        match &self.instance_pre {
            SubtitleInstancePre::V1_0(_) => SubtitleContractVersion::V1_0,
            SubtitleInstancePre::V1_1(_) => SubtitleContractVersion::V1_1,
        }
    }

    async fn instantiate(
        &self,
        wasi: WasiCtx,
        command_host: CommandHost,
        memory_max_bytes: Option<usize>,
        timeout: Duration,
    ) -> Result<(Store<SubtitleComponentCtx>, SubtitlePlugin), wasmtime::Error> {
        let clock_origin = Instant::now();
        let mut store = Store::new(
            self.component.engine(),
            SubtitleComponentCtx {
                table: ResourceTable::new(),
                wasi,
                limits: HostLimits::new(memory_max_bytes),
                command_host,
                clock_origin,
                deadline: clock_origin + timeout,
            },
        );
        store.limiter(|ctx: &mut SubtitleComponentCtx| &mut ctx.limits);
        store.set_epoch_deadline(engine::deadline_ticks(timeout));
        let plugin = match &self.instance_pre {
            SubtitleInstancePre::V1_0(instance_pre) => instance_pre
                .instantiate_async(&mut store)
                .await
                .map(SubtitlePlugin::V1_0)?,
            SubtitleInstancePre::V1_1(instance_pre) => instance_pre
                .instantiate_async(&mut store)
                .await
                .map(SubtitlePlugin::V1_1)?,
        };
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
    let descriptor_json = call_describe(&mut store, &plugin).await.map_err(|error| {
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

    let contract = runtime.contract_version();

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

    let call_result = call_process(&mut store, &plugin, request_bytes).await;
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
        Ok(Err(label)) => {
            let failure = error::protocol_failure(format!("subtitle component reported {label}"));
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
        contract = contract.as_str(),
        duration_ms = started.elapsed().as_millis() as u64,
        request_bytes = request_len,
        response_bytes = response_bytes.len(),
        disposition = "ok",
        "subtitle plugin invocation complete",
    );

    Ok(response)
}

/// Call `describe` on whichever contract the artifact was bound to.
///
/// `describe` is a plain synchronous export in both revisions, so neither
/// version needs the concurrent store; only the generated type differs.
async fn call_describe(
    store: &mut Store<SubtitleComponentCtx>,
    plugin: &SubtitlePlugin,
) -> Result<Vec<u8>, wasmtime::Error> {
    match plugin {
        SubtitlePlugin::V1_0(plugin) => plugin.call_describe(&mut *store).await,
        SubtitlePlugin::V1_1(plugin) => plugin.call_describe(&mut *store).await,
    }
}

/// Call `process` on whichever contract the artifact was bound to.
///
/// The two revisions differ in exactly one way that reaches this function: 1.0
/// exports a synchronous `process` that is driven directly on the store, while
/// 1.1 exports an `async func` whose task must be driven by the store's
/// concurrent scheduler, so the guest can await `http` and `sleep` without
/// blocking the host worker. Both collapse to the same
/// `Result<Result<bytes, label>, wasmtime::Error>` so the caller's failure
/// handling stays single-path.
async fn call_process(
    store: &mut Store<SubtitleComponentCtx>,
    plugin: &SubtitlePlugin,
    request: Vec<u8>,
) -> Result<Result<Vec<u8>, &'static str>, wasmtime::Error> {
    match plugin {
        SubtitlePlugin::V1_0(plugin) => plugin
            .call_process(&mut *store, &request)
            .await
            .map(|result| result.map_err(invocation_error_label)),
        SubtitlePlugin::V1_1(plugin) => store
            .run_concurrent(async move |accessor| plugin.call_process(accessor, request).await)
            .await?
            .map(|result| result.map_err(invocation_error_label_v1_1)),
    }
}

const fn invocation_error_label(error: InvocationError) -> &'static str {
    match error {
        InvocationError::Failed => "failed",
        InvocationError::Cancelled => "cancelled",
        InvocationError::InvalidResponse => "invalid-response",
    }
}

const fn invocation_error_label_v1_1(error: contract_v1_1::InvocationError) -> &'static str {
    match error {
        contract_v1_1::InvocationError::Failed => "failed",
        contract_v1_1::InvocationError::Cancelled => "cancelled",
        contract_v1_1::InvocationError::InvalidResponse => "invalid-response",
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

    /// Guest memory layout for the 1.1 fixture component below.
    const V1_1_DESCRIPTOR_PTR: usize = 0;
    const V1_1_OK_RESPONSE_PTR: usize = 8192;
    const V1_1_FAIL_RESPONSE_PTR: usize = 16384;
    const V1_1_METHOD_PTR: usize = 24576;
    const V1_1_URL_PTR: usize = 24608;
    const V1_1_STATE_KEY_PTR: usize = 24704;
    const V1_1_STATE_VALUE_PTR: usize = 24736;
    /// A 32-byte, 4-aligned `http-request` record.
    const V1_1_HTTP_ARGS_PTR: usize = 24768;
    /// A 24-byte, 4-aligned `result<http-response, transport-error>`.
    const V1_1_HTTP_RESULT_PTR: usize = 24832;
    const V1_1_DESCRIBE_RETURN_PTR: usize = 25600;

    /// The host the 1.1 fixture asks for. It is deliberately absent from the
    /// fixture's allowed hosts, so the typed `http` import must come back as an
    /// error rather than reaching the network from a unit test.
    const V1_1_FIXTURE_URL: &str = "https://fixture.invalid/subtitles";
    const V1_1_STATE_KEY: &str = "cursor";
    const V1_1_STATE_VALUE: &str = "1";

    /// The 1.1 world's `transport-error` discriminants, in declaration order.
    const V1_1_FORBIDDEN_ORIGIN: u8 = 1;

    fn subtitle_descriptor_json_v1_1() -> String {
        subtitle_descriptor_json().replace("fixture-subtitle", "fixture-subtitle-1-1")
    }

    /// The document the 1.1 guest returns once every runtime-host observation
    /// held.
    fn ok_response_json_v1_1() -> String {
        serde_json::to_string(&PluginCommandResponse::new(PluginCommandResult::Subtitle(
            PluginSubtitleCommandResult::Search(PluginResult::Ok(SubtitlePluginSearchResponse {
                results: vec![SubtitlePluginCandidate {
                    provider_file_id: "runtime-host:forbidden+cas".to_string(),
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

    fn fail_response_json_v1_1() -> String {
        serde_json::to_string(&PluginCommandResponse::new(PluginCommandResult::Subtitle(
            PluginSubtitleCommandResult::Search(PluginResult::Err(
                scryer_plugin_sdk::PluginError {
                    code: scryer_plugin_sdk::PluginErrorCode::Permanent,
                    public_message: "runtime host binding mismatch".to_string(),
                    debug_message: None,
                    retry_after_seconds: None,
                    details: None,
                },
            )),
        )))
        .expect("fixture fail response must serialize")
    }

    /// A minimal but real `scryer:subtitle/subtitle-provider@1.1.0` component.
    ///
    /// This is the async half of the contract exercised end to end without a
    /// guest toolchain in the loop. `process` is `canon lift ... async` with a
    /// callback, which is the shape wit-bindgen emits, so the export really is
    /// driven through the store's concurrent scheduler rather than called
    /// straight through.
    ///
    /// The guest gates its answer on three facts a mis-wired host cannot make
    /// true at once:
    ///
    /// 1. the command envelope arrived as JSON (first byte `{`);
    /// 2. two `state-cas` calls against the *same* key both from `none`: the
    ///    first must win and the second must lose. Get-then-set would let both
    ///    win, so this is the atomicity assertion, not a smoke test;
    /// 3. one typed `http` call to a host outside the plugin's allowed hosts
    ///    comes back `err(forbidden-origin)` — proof the typed import is
    ///    projected onto the same policy layer the encoded door uses, rather
    ///    than onto a second HTTP client.
    ///
    /// The `http` import is `canon lower ... async`, so the call really does
    /// suspend the guest task: the fixture stores the returned subtask in a
    /// waitable set, returns the `WAIT` callback code, and finishes inside its
    /// callback. A host that answered `http` synchronously would take the
    /// early-return branch instead and still pass, which is intended — the
    /// fixture asserts the answer, not the scheduling path.
    #[allow(clippy::too_many_arguments)]
    fn fixture_component_v1_1_wat(
        descriptor_json: &str,
        ok_json: &str,
        fail_json: &str,
        expected_transport_error: u8,
    ) -> String {
        format!(
            r#"(component
  (import "scryer:runtime/host@1.0.0" (instance $rt
    ;; Every type an imported instance type mentions has to be spelled out with
    ;; its own index, including the anonymous `list` and `option` types: an
    ;; inline `(list 1)` inside a record silently claims the next type index and
    ;; shifts every `(type (eq N))` after it, which is how you end up exporting
    ;; `list<header>` under the name `http-request`.
    ;;  0 header
    (type (record (field "name" string) (field "value" string)))
    ;;  1 exported alias for `header`
    (export "header" (type (eq 0)))
    ;;  2 list<header>
    (type (list 1))
    ;;  3 list<u8>
    (type (list u8))
    ;;  4 http-request
    (type (record
      (field "method" string)
      (field "url" string)
      (field "headers" 2)
      (field "body" 3)))
    ;;  5 exported alias for `http-request`
    (export "http-request" (type (eq 4)))
    ;;  6 http-response
    (type (record
      (field "status" u16)
      (field "headers" 2)
      (field "body" 3)))
    ;;  7 exported alias for `http-response`
    (export "http-response" (type (eq 6)))
    ;;  8 transport-error
    (type (enum "invalid-request" "forbidden-origin" "timeout" "cancelled"
                "response-too-large" "capacity" "transport"))
    ;;  9 exported alias for `transport-error`
    (export "transport-error" (type (eq 8)))
    ;; 10 result<http-response, transport-error>
    (type (result 7 (error 9)))
    ;; 11 option<list<u8>>
    (type (option 3))
    (export "http" (func async (param "request" 5) (result 10)))
    (export "state-cas" (func
      (param "key" string)
      (param "expected" 11)
      (param "replacement" 11)
      (result bool)))
  ))

  (type $ie (enum "failed" "cancelled" "invalid-response"))
  (export $ieX "invocation-error" (type $ie))
  (type $describe-ty (func (result (list u8))))
  (type $process-ty (func async (param "request" (list u8))
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

  (core func $http_low
    (canon lower (func $rt "http") async (memory $mem) (realloc $realloc)))
  (core func $cas_low
    (canon lower (func $rt "state-cas") (memory $mem) (realloc $realloc)))
  (core func $task_return
    (canon task.return (result (result (list u8) (error $ieX))) (memory $mem)))
  (core func $wsn (canon waitable-set.new))
  (core func $wsd (canon waitable-set.drop))
  (core func $wj (canon waitable.join))
  (core func $sd (canon subtask.drop))

  (core module $main
    (import "libc" "memory" (memory 2))
    (import "host" "http" (func $http (param i32 i32) (result i32)))
    (import "host" "cas" (func $cas
      (param i32 i32 i32 i32 i32 i32 i32 i32) (result i32)))
    (import "host" "return" (func $ret (param i32 i32 i32)))
    (import "host" "wsn" (func $wsn (result i32)))
    (import "host" "wsd" (func $wsd (param i32)))
    (import "host" "wj" (func $wj (param i32 i32)))
    (import "host" "sd" (func $sd (param i32)))

    (data (i32.const {descriptor_ptr}) "{descriptor}")
    (data (i32.const {ok_ptr}) "{ok}")
    (data (i32.const {fail_ptr}) "{fail}")
    (data (i32.const {method_ptr}) "GET")
    (data (i32.const {url_ptr}) "{url}")
    (data (i32.const {key_ptr}) "{key}")
    (data (i32.const {value_ptr}) "{value}")

    (global $ok (mut i32) (i32.const 1))
    (global $set (mut i32) (i32.const 0))

    (func (export "describe") (result i32)
      (i32.store (i32.const {describe_ret}) (i32.const {descriptor_ptr}))
      (i32.store (i32.const {describe_ret_len}) (i32.const {descriptor_len}))
      (i32.const {describe_ret}))

    (func $finish
      (if (global.get $ok)
        (then (call $ret (i32.const 0) (i32.const {ok_ptr}) (i32.const {ok_len})))
        (else (call $ret (i32.const 0) (i32.const {fail_ptr}) (i32.const {fail_len})))))

    ;; The typed http answer must be err(<expected transport-error>).
    (func $check_http
      (if (i32.ne (i32.load8_u (i32.const {http_result_ptr})) (i32.const 1))
        (then (global.set $ok (i32.const 0))))
      (if (i32.ne (i32.load8_u (i32.const {http_error_ptr}))
                  (i32.const {expected_transport_error}))
        (then (global.set $ok (i32.const 0)))))

    (func (export "process") (param $ptr i32) (param $len i32) (result i32)
      (local $status i32)
      (local $subtask i32)
      (if (i32.eqz (local.get $len))
        (then (global.set $ok (i32.const 0)))
        (else (if (i32.ne (i32.load8_u (local.get $ptr)) (i32.const 123))
          (then (global.set $ok (i32.const 0))))))
      ;; A compare-and-swap from absent must win exactly once.
      (if (i32.ne
            (call $cas (i32.const {key_ptr}) (i32.const {key_len})
              (i32.const 0) (i32.const 0) (i32.const 0)
              (i32.const 1) (i32.const {value_ptr}) (i32.const {value_len}))
            (i32.const 1))
        (then (global.set $ok (i32.const 0))))
      (if (i32.ne
            (call $cas (i32.const {key_ptr}) (i32.const {key_len})
              (i32.const 0) (i32.const 0) (i32.const 0)
              (i32.const 1) (i32.const {value_ptr}) (i32.const {value_len}))
            (i32.const 0))
        (then (global.set $ok (i32.const 0))))
      ;; One typed http request, lowered async.
      (i32.store (i32.const {http_args_ptr}) (i32.const {method_ptr}))
      (i32.store (i32.const {http_args_method_len}) (i32.const 3))
      (i32.store (i32.const {http_args_url_ptr}) (i32.const {url_ptr}))
      (i32.store (i32.const {http_args_url_len}) (i32.const {url_len}))
      (i32.store (i32.const {http_args_headers_ptr}) (i32.const 0))
      (i32.store (i32.const {http_args_headers_len}) (i32.const 0))
      (i32.store (i32.const {http_args_body_ptr}) (i32.const 0))
      (i32.store (i32.const {http_args_body_len}) (i32.const 0))
      (local.set $status
        (call $http (i32.const {http_args_ptr}) (i32.const {http_result_ptr})))
      ;; status 2 is `returned`: the host answered without suspending us.
      (if (i32.eq (i32.and (local.get $status) (i32.const 15)) (i32.const 2))
        (then
          (call $check_http)
          (call $finish)
          (return (i32.const 0))))
      (local.set $subtask (i32.shr_u (local.get $status) (i32.const 4)))
      (global.set $set (call $wsn))
      (call $wj (local.get $subtask) (global.get $set))
      ;; callback code 2 is `wait`, packed with the waitable set to wait on.
      (i32.or (i32.const 2) (i32.shl (global.get $set) (i32.const 4))))

    (func (export "callback") (param $event i32) (param $waitable i32)
      (param $code i32) (result i32)
      (call $sd (local.get $waitable))
      (call $wsd (global.get $set))
      (call $check_http)
      (call $finish)
      ;; callback code 0 is `exit`.
      (i32.const 0))
  )
  (core instance $maini (instantiate $main
    (with "libc" (instance $libci))
    (with "host" (instance
      (export "http" (func $http_low))
      (export "cas" (func $cas_low))
      (export "return" (func $task_return))
      (export "wsn" (func $wsn))
      (export "wsd" (func $wsd))
      (export "wj" (func $wj))
      (export "sd" (func $sd))))))

  (alias core export $maini "callback" (core func $cb))
  (func (export "describe") (type $describe-ty)
    (canon lift (core func $maini "describe") (memory $mem) (realloc $realloc)))
  (func (export "process") (type $process-ty)
    (canon lift (core func $maini "process") async (callback $cb)
      (memory $mem) (realloc $realloc)))
)"#,
            descriptor_ptr = V1_1_DESCRIPTOR_PTR,
            descriptor = descriptor_json.replace('"', "\\\""),
            descriptor_len = descriptor_json.len(),
            ok_ptr = V1_1_OK_RESPONSE_PTR,
            ok = ok_json.replace('"', "\\\""),
            ok_len = ok_json.len(),
            fail_ptr = V1_1_FAIL_RESPONSE_PTR,
            fail = fail_json.replace('"', "\\\""),
            fail_len = fail_json.len(),
            method_ptr = V1_1_METHOD_PTR,
            url_ptr = V1_1_URL_PTR,
            url = V1_1_FIXTURE_URL,
            url_len = V1_1_FIXTURE_URL.len(),
            key_ptr = V1_1_STATE_KEY_PTR,
            key = V1_1_STATE_KEY,
            key_len = V1_1_STATE_KEY.len(),
            value_ptr = V1_1_STATE_VALUE_PTR,
            value = V1_1_STATE_VALUE,
            value_len = V1_1_STATE_VALUE.len(),
            http_args_ptr = V1_1_HTTP_ARGS_PTR,
            http_args_method_len = V1_1_HTTP_ARGS_PTR + 4,
            http_args_url_ptr = V1_1_HTTP_ARGS_PTR + 8,
            http_args_url_len = V1_1_HTTP_ARGS_PTR + 12,
            http_args_headers_ptr = V1_1_HTTP_ARGS_PTR + 16,
            http_args_headers_len = V1_1_HTTP_ARGS_PTR + 20,
            http_args_body_ptr = V1_1_HTTP_ARGS_PTR + 24,
            http_args_body_len = V1_1_HTTP_ARGS_PTR + 28,
            http_result_ptr = V1_1_HTTP_RESULT_PTR,
            http_error_ptr = V1_1_HTTP_RESULT_PTR + 4,
            describe_ret = V1_1_DESCRIBE_RETURN_PTR,
            describe_ret_len = V1_1_DESCRIBE_RETURN_PTR + 4,
        )
    }

    pub(crate) fn fixture_component_v1_1() -> Vec<u8> {
        wat::parse_str(fixture_component_v1_1_wat(
            &subtitle_descriptor_json_v1_1(),
            &ok_response_json_v1_1(),
            &fail_response_json_v1_1(),
            V1_1_FORBIDDEN_ORIGIN,
        ))
        .expect("fixture subtitle 1.1 component WAT must assemble")
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
            scryer_outbound_http::PluginEgressPolicy::default(),
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

    fn search_result(
        response: PluginCommandResponse,
    ) -> PluginResult<SubtitlePluginSearchResponse> {
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
        // The dual-contract loader reports both attempts, so the message names
        // each world it tried and why the component did not fit it.
        assert!(
            error.contains("scryer:subtitle/subtitle-provider@1.1.0")
                && error.contains("@1.0.0")
                && error.contains("describe"),
            "{error}"
        );
    }

    #[test]
    fn the_1_1_fixture_component_passes_world_validation() {
        validate_subtitle_component(&fixture_component_v1_1())
            .expect("the 1.1 fixture must satisfy scryer:subtitle/subtitle-provider@1.1.0");
    }

    /// The host must tell the two revisions apart on the artifact alone: the
    /// same linker serves both, and the newer contract is tried first.
    #[test]
    fn each_fixture_binds_to_its_own_contract() {
        let v1_0 =
            SubtitleComponentRuntime::new(engine::shared_async_engine(), &fixture_component())
                .expect("the 1.0 fixture must bind");
        assert_eq!(v1_0.contract_version(), SubtitleContractVersion::V1_0);

        let v1_1 =
            SubtitleComponentRuntime::new(engine::shared_async_engine(), &fixture_component_v1_1())
                .expect("the 1.1 fixture must bind");
        assert_eq!(v1_1.contract_version(), SubtitleContractVersion::V1_1);
    }

    #[test]
    fn describe_returns_the_guest_descriptor_on_contract_1_1() {
        let descriptor = subtitle_component_describe(&fixture_component_v1_1())
            .expect("the 1.1 fixture must self-describe through the world's describe export");

        assert_eq!(descriptor.id, "fixture-subtitle-1-1");
    }

    /// The 1.1 end-to-end path: an async `process` export driven through the
    /// store's concurrent scheduler, a typed `http` import that lands on the
    /// same allowed-hosts policy the encoded door enforces, and a `state-cas`
    /// that is genuinely atomic against the plugin's own state map.
    #[tokio::test(flavor = "multi_thread")]
    async fn process_drives_the_async_export_and_the_typed_runtime_imports() {
        let spec = test_spec(fixture_component_v1_1(), configured_command_host());

        let response = process_subtitle_component(&spec, &search_command_request(), invocation())
            .await
            .expect("the 1.1 fixture component must complete one process exchange");

        let PluginResult::Ok(search) = search_result(response) else {
            panic!(
                "a plugin error means a typed runtime import did not behave as the world promises",
            );
        };
        assert_eq!(search.results.len(), 1);
        assert_eq!(
            search.results[0].provider_file_id,
            "runtime-host:forbidden+cas"
        );
    }

    /// The negative half of the assertion above: a guest that expects the wrong
    /// `transport-error` takes its failure branch, so the success is about the
    /// projection onto the policy layer and not about `http` merely returning.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_typed_http_answer_the_guest_did_not_expect_drives_the_failure_branch() {
        let wasm = wat::parse_str(fixture_component_v1_1_wat(
            &subtitle_descriptor_json_v1_1(),
            &ok_response_json_v1_1(),
            &fail_response_json_v1_1(),
            // `timeout`, which this request cannot produce.
            2,
        ))
        .expect("fixture subtitle 1.1 component WAT must assemble");
        let spec = test_spec(wasm, configured_command_host());

        let response = process_subtitle_component(&spec, &search_command_request(), invocation())
            .await
            .expect("the 1.1 fixture component must still complete the exchange");

        let PluginResult::Err(error) = search_result(response) else {
            panic!("a mismatched transport error must not produce a successful search");
        };
        assert!(
            error
                .public_message
                .contains("runtime host binding mismatch"),
            "{}",
            error.public_message
        );
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
