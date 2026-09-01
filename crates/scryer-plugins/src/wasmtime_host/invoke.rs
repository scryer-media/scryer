//! Command-protocol invocation for Wasmtime command-model plugins.
//!
//! Instance-per-request, WASI-command style: the serialized
//! request is fed on stdin, the guest writes exactly one typed response JSON
//! document to stdout, and a clean `_start` return (or `proc_exit(0)`) marks
//! protocol success. Operational conditions stay in-band via the response; only
//! genuine faults exit non-zero / trap. Command-model guests run on Wasmtime's
//! async path so WASI sleeps and polls can be cancelled by adapter timeouts
//! without parking Tokio blocking threads.

use std::sync::Arc;
use std::time::{Duration, Instant};

use scryer_application::{AppError, AppResult};
use scryer_plugin_sdk::SubtitleSyncPluginProcessResponse;
use scryer_plugin_sdk::command::{PluginCommandRequest, PluginCommandResponse};
use wasmtime::{Linker, Module, Store};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;

use crate::runtime_backing::PluginInstanceSpec;
use crate::wasmtime_host::sandbox::{self, HostCtx, HostLimits, PreparedSandbox};
use crate::wasmtime_host::{command_host, engine, error, module_cache};

/// Amount of guest stderr forwarded to tracing / attached to error messages.
const STDERR_TAIL_BYTES: usize = 8 * 1024;

/// Identifying context for one subtitle-sync command invocation.
pub(crate) struct SubtitleSyncInvocation<'a> {
    pub(crate) plugin_id: &'a str,
    pub(crate) plugin_version: &'a str,
    pub(crate) operation: &'a str,
}

/// Identifying context for one marker-selected command invocation.
pub(crate) struct CommandInvocation<'a> {
    pub(crate) plugin_id: &'a str,
    pub(crate) plugin_version: &'a str,
    pub(crate) operation: &'a str,
}

async fn prepare_command_module(
    wasm: Arc<Vec<u8>>,
    timeout: Duration,
) -> Result<Arc<Module>, String> {
    let prepare = tokio::task::spawn_blocking(move || module_cache::command_module(&wasm));
    match tokio::time::timeout(timeout, prepare).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(format!("plugin module preparation task failed: {error}")),
        Err(_) => Err(format!(
            "timed out waiting for plugin module rehydration after {} ms",
            timeout.as_millis()
        )),
    }
}

/// Instantiate a marker-selected native command and run one typed exchange.
///
/// The command runner is shared by every descriptor adapter.  It deliberately
/// has no legacy export fallback: a marked artifact either speaks the command
/// protocol or fails as a protocol error.
pub(crate) async fn process_command(
    spec: &PluginInstanceSpec,
    request: &PluginCommandRequest,
    invocation: CommandInvocation<'_>,
) -> AppResult<PluginCommandResponse> {
    let started = Instant::now();
    let request_bytes = serde_json::to_vec(request).map_err(|error| {
        AppError::Repository(format!(
            "failed to serialize native plugin command: {error}"
        ))
    })?;
    let request_len = request_bytes.len();
    let engine = engine::shared_async_engine();
    let module = prepare_command_module(Arc::clone(&spec.wasm), spec.timeout)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "command plugin {}@{} failed to prepare: {error}",
                invocation.plugin_id, invocation.plugin_version
            ))
        })?;

    let mut linker: Linker<HostCtx> = Linker::new(engine);
    wasmtime_wasi::p1::add_to_linker_async(&mut linker, |ctx: &mut HostCtx| &mut ctx.wasi)
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to wire WASI preview1 for command plugin: {error:#}"
            ))
        })?;
    command_host::add_to_linker(&mut linker).map_err(|error| {
        AppError::Repository(format!(
            "failed to register native command host functions: {error:#}"
        ))
    })?;
    let PreparedSandbox {
        wasi,
        stdout,
        stderr,
        _scratch,
    } = sandbox::build_sandbox(&spec.preopens, request_bytes)?;
    let mut store = Store::new(
        engine,
        HostCtx::with_command_host(
            wasi,
            HostLimits::new(spec.memory_max_bytes),
            spec.command_host.for_invocation(spec.timeout),
        ),
    );
    store.limiter(|ctx: &mut HostCtx| &mut ctx.limits);
    store.set_epoch_deadline(engine::deadline_ticks(spec.timeout));

    let instance = linker
        .instantiate_async(&mut store, &module)
        .await
        .map_err(|error| {
            let failure = error::classify_error(&error, store.data().limits.memory_denied);
            finish_command_error(
                &invocation,
                spec.timeout,
                &tail_of(&stderr),
                &failure,
                started,
                request_len,
            )
        })?;
    let start = instance
        .get_typed_func::<(), ()>(&mut store, "_start")
        .map_err(|error| {
            let failure = error::protocol_failure(format!(
                "guest is not a wasip1 command (missing _start): {error:#}"
            ));
            finish_command_error(
                &invocation,
                spec.timeout,
                &tail_of(&stderr),
                &failure,
                started,
                request_len,
            )
        })?;
    let call_result = start.call_async(&mut store, ()).await;
    let denied = store.data().limits.memory_denied;
    let stdout_bytes = stdout.contents();
    let stderr_tail = tail_of(&stderr);

    // Command guests have no host log service; stderr is their diagnostic
    // channel. The error paths below already attach it, but a plugin that
    // succeeds while logging (every migrated indexer does) would otherwise have
    // its output silently dropped.
    if !stderr_tail.is_empty() {
        tracing::debug!(
            target: "scryer_plugins::command",
            plugin_id = invocation.plugin_id,
            plugin_version = invocation.plugin_version,
            operation = invocation.operation,
            stderr = stderr_tail.as_str(),
            "command plugin stderr",
        );
    }

    if let Err(failure) = error::interpret_start_result(call_result, denied) {
        return Err(finish_command_error(
            &invocation,
            spec.timeout,
            &stderr_tail,
            &failure,
            started,
            request_len,
        ));
    }
    let response: PluginCommandResponse =
        serde_json::from_slice(&stdout_bytes).map_err(|error| {
            let failure = error::protocol_failure(format!(
                "stdout was not valid PluginCommandResponse JSON: {error}"
            ));
            finish_command_error(
                &invocation,
                spec.timeout,
                &stderr_tail,
                &failure,
                started,
                request_len,
            )
        })?;
    if response.abi_version != scryer_plugin_sdk::command::COMMAND_ABI_VERSION {
        let failure = error::protocol_failure(format!(
            "command response used unsupported ABI version {}",
            response.abi_version
        ));
        return Err(finish_command_error(
            &invocation,
            spec.timeout,
            &stderr_tail,
            &failure,
            started,
            request_len,
        ));
    }
    Ok(response)
}

/// Instantiate the subtitle-sync guest and run one request→response exchange.
pub(crate) async fn process_subtitle_sync(
    spec: &PluginInstanceSpec,
    request_json: &str,
    invocation: SubtitleSyncInvocation<'_>,
) -> AppResult<SubtitleSyncPluginProcessResponse> {
    let span = tracing::info_span!(
        "subtitle_sync_plugin_invoke",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
    );
    let _enter = span.enter();

    let started = Instant::now();
    let request_bytes = request_json.as_bytes().to_vec();
    let request_len = request_bytes.len();

    let engine = engine::shared_async_engine();
    let module = prepare_command_module(Arc::clone(&spec.wasm), spec.timeout)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "subtitle sync plugin {}@{} failed to prepare: {error}",
                invocation.plugin_id, invocation.plugin_version
            ))
        })?;

    let mut linker: Linker<HostCtx> = Linker::new(engine);
    wasmtime_wasi::p1::add_to_linker_async(&mut linker, |ctx: &mut HostCtx| &mut ctx.wasi)
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to wire WASI preview1 for subtitle sync plugin: {error:#}"
            ))
        })?;

    let PreparedSandbox {
        wasi,
        stdout,
        stderr,
        _scratch,
    } = sandbox::build_sandbox(&spec.preopens, request_bytes)?;

    let mut store = Store::new(
        engine,
        HostCtx::new(wasi, HostLimits::new(spec.memory_max_bytes)),
    );
    store.limiter(|ctx: &mut HostCtx| &mut ctx.limits);
    store.set_epoch_deadline(engine::deadline_ticks(spec.timeout));

    let instance = match linker.instantiate_async(&mut store, &module).await {
        Ok(instance) => instance,
        Err(error) => {
            let denied = store.data().limits.memory_denied;
            let failure = error::classify_error(&error, denied);
            return Err(finish_subtitle_sync_error(
                &invocation,
                spec.timeout,
                &tail_of(&stderr),
                &failure,
                started,
                request_len,
            ));
        }
    };

    let start = match instance.get_typed_func::<(), ()>(&mut store, "_start") {
        Ok(start) => start,
        Err(error) => {
            let failure = error::protocol_failure(format!(
                "guest is not a wasip1 command (missing _start): {error:#}"
            ));
            return Err(finish_subtitle_sync_error(
                &invocation,
                spec.timeout,
                &tail_of(&stderr),
                &failure,
                started,
                request_len,
            ));
        }
    };

    let call_result = start.call_async(&mut store, ()).await;
    let denied = store.data().limits.memory_denied;
    let stdout_bytes = stdout.contents();
    let stderr_tail = tail_of(&stderr);

    if !stderr_tail.is_empty() {
        tracing::debug!(
            target: "scryer_plugins::subtitle_sync",
            plugin_id = invocation.plugin_id,
            stderr = stderr_tail.as_str(),
            "subtitle sync plugin stderr",
        );
    }

    if let Err(failure) = error::interpret_start_result(call_result, denied) {
        return Err(finish_subtitle_sync_error(
            &invocation,
            spec.timeout,
            &stderr_tail,
            &failure,
            started,
            request_len,
        ));
    }

    let response: SubtitleSyncPluginProcessResponse = match serde_json::from_slice(&stdout_bytes) {
        Ok(response) => response,
        Err(error) => {
            let failure = error::protocol_failure(format!(
                "stdout was not a valid SubtitleSyncPluginProcessResponse JSON: {error}"
            ));
            return Err(finish_subtitle_sync_error(
                &invocation,
                spec.timeout,
                &stderr_tail,
                &failure,
                started,
                request_len,
            ));
        }
    };

    let duration_ms = started.elapsed().as_millis() as u64;
    let response_bytes = stdout_bytes.len();
    tracing::debug!(
        target: "scryer_plugins::subtitle_sync",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
        duration_ms,
        request_bytes = request_len,
        response_bytes,
        disposition = "ok",
        "subtitle sync plugin invocation complete",
    );

    Ok(response)
}

fn finish_command_error(
    invocation: &CommandInvocation<'_>,
    budget: Duration,
    stderr_tail: &str,
    failure: &error::RunFailure,
    started: Instant,
    request_len: usize,
) -> AppError {
    tracing::debug!(
        target: "scryer_plugins::command",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
        duration_ms = started.elapsed().as_millis() as u64,
        request_bytes = request_len,
        disposition = ?failure.kind,
        "native command plugin invocation failed",
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

/// Log the failing subtitle-sync disposition and build the operator-facing `AppError`.
fn finish_subtitle_sync_error(
    invocation: &SubtitleSyncInvocation<'_>,
    budget: Duration,
    stderr_tail: &str,
    failure: &error::RunFailure,
    started: Instant,
    request_len: usize,
) -> AppError {
    let duration_ms = started.elapsed().as_millis() as u64;
    let disposition = format!("{:?}", failure.kind);
    tracing::debug!(
        target: "scryer_plugins::subtitle_sync",
        plugin_id = invocation.plugin_id,
        plugin_version = invocation.plugin_version,
        operation = invocation.operation,
        duration_ms,
        request_bytes = request_len,
        disposition,
        "subtitle sync plugin invocation failed",
    );
    let ctx = error::InvocationContext {
        plugin_id: invocation.plugin_id,
        plugin_version: invocation.plugin_version,
        operation: invocation.operation,
        budget,
        stderr_tail,
    };
    error::to_app_error(failure, &ctx)
}

/// Size-capped, lossy tail of a captured output pipe.
fn tail_of(pipe: &MemoryOutputPipe) -> String {
    let bytes = pipe.contents();
    let start = bytes.len().saturating_sub(STDERR_TAIL_BYTES);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use wasmtime::Engine;

    /// A wasip1 command that echoes up to 1 KiB of stdin to stdout.
    const ECHO_WAT: &str = r#"
        (module
          (import "wasi_snapshot_preview1" "fd_read"
            (func $fd_read (param i32 i32 i32 i32) (result i32)))
          (import "wasi_snapshot_preview1" "fd_write"
            (func $fd_write (param i32 i32 i32 i32) (result i32)))
          (memory (export "memory") 1)
          (func (export "_start")
            ;; read iovec @0: base=16, len=1024
            (i32.store (i32.const 0) (i32.const 16))
            (i32.store (i32.const 4) (i32.const 1024))
            (drop (call $fd_read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 8)))
            ;; write iovec @0: base=16, len=*nread(@8)
            (i32.store (i32.const 0) (i32.const 16))
            (i32.store (i32.const 4) (i32.load (i32.const 8)))
            (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 8)))))
    "#;

    /// A guest whose `_start` spins forever — for the epoch-deadline test.
    const SPIN_WAT: &str = r#"(module (func (export "_start") (loop br 0)))"#;

    /// A wasip1 command that parks in `poll_oneoff` for five seconds.
    const POLL_ONEOFF_SLEEP_WAT: &str = r#"
        (module
          (import "wasi_snapshot_preview1" "poll_oneoff"
            (func $poll_oneoff (param i32 i32 i32 i32) (result i32)))
          (memory (export "memory") 1)
          (func (export "_start")
            ;; __wasi_subscription_t @0: userdata=1, type=clock,
            ;; clockid=monotonic, timeout=5s, precision=0, flags=0.
            (i64.store (i32.const 0) (i64.const 1))
            (i32.store8 (i32.const 8) (i32.const 0))
            (i32.store (i32.const 16) (i32.const 1))
            (i64.store (i32.const 24) (i64.const 5000000000))
            (i64.store (i32.const 32) (i64.const 0))
            (i32.store16 (i32.const 40) (i32.const 0))
            (drop (call $poll_oneoff
              (i32.const 0)
              (i32.const 64)
              (i32.const 1)
              (i32.const 128)))))
    "#;

    /// A guest demanding 100 pages (6.4 MiB) of initial memory — for the cap.
    const BIG_MEM_WAT: &str = r#"(module (memory (export "memory") 100))"#;

    fn module_from_wat(engine: &Engine, wat: &str, context: &str) -> Module {
        let wasm = wat::parse_str(wat).unwrap_or_else(|error| panic!("{context}: {error}"));
        Module::new(engine, wasm).unwrap_or_else(|error| panic!("{context}: {error}"))
    }

    /// PROTOCOL GATE: request-on-stdin / response-on-stdout capture
    /// under wasmtime-wasi p1 with a `Store<HostCtx>`. If this fails, the host
    /// (and the PDK) must fall back to control files.
    #[test]
    fn stdin_stdout_round_trips_under_wasi_p1() {
        let engine = Engine::default();
        let module = module_from_wat(&engine, ECHO_WAT, "compile echo guest");
        let mut linker: Linker<HostCtx> = Linker::new(&engine);
        wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |ctx: &mut HostCtx| &mut ctx.wasi)
            .unwrap();

        let request = br#"{"operation":{"Inspect":{"source_dir":"/scryer/source"}}}"#.to_vec();
        let PreparedSandbox {
            wasi,
            stdout,
            stderr: _,
            _scratch,
        } = sandbox::build_sandbox(&[], request.clone()).expect("build sandbox");

        let mut store = Store::new(&engine, HostCtx::new(wasi, HostLimits::new(None)));
        let instance = linker.instantiate(&mut store, &module).unwrap();
        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .unwrap();
        start.call(&mut store, ()).expect("echo guest runs cleanly");

        assert_eq!(
            stdout.contents().as_ref(),
            request.as_slice(),
            "stdin must round-trip to captured stdout under wasmtime-wasi p1"
        );
    }

    /// A spinning guest must be cancelled by the epoch deadline (using the real
    /// process-wide engine + ticker) and map to a timeout failure.
    #[test]
    fn spinning_guest_hits_epoch_deadline() {
        let engine = engine::shared_engine();
        let module = module_from_wat(engine, SPIN_WAT, "compile spin guest");
        let linker: Linker<()> = Linker::new(engine);
        let mut store = Store::new(engine, ());
        // One tick: the ~100ms background ticker advances the epoch and fires.
        store.set_epoch_deadline(engine::deadline_ticks(Duration::from_millis(1)));
        let instance = linker
            .instantiate(&mut store, &module)
            .expect("instantiate spin guest");
        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .unwrap();

        let result = start.call(&mut store, ());
        let failure = error::interpret_start_result(result, false)
            .expect_err("spinning guest must be interrupted");
        assert_eq!(failure.kind, error::FailureKind::Timeout);
    }

    #[test]
    fn poll_oneoff_timeout_does_not_hold_blocking_thread() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .max_blocking_threads(1)
            .enable_time()
            .build()
            .expect("build constrained tokio runtime");

        runtime.block_on(async {
            let spec = PluginInstanceSpec {
                wasm: Arc::new(wat::parse_str(POLL_ONEOFF_SLEEP_WAT).expect("compile sleep guest")),
                preopens: Vec::new(),
                timeout: Duration::from_secs(30),
                memory_max_bytes: None,
                command_host: crate::wasmtime_host::command_host::CommandHost::disabled(),
            };
            let invocation = SubtitleSyncInvocation {
                plugin_id: "sleepy",
                plugin_version: "1.0.0",
                operation: "Sync",
            };

            let timed = tokio::time::timeout(
                Duration::from_millis(100),
                process_subtitle_sync(&spec, "{}", invocation),
            )
            .await;

            assert!(
                timed.is_err(),
                "guest poll_oneoff should be cancelled by the adapter timeout"
            );

            let sentinel = tokio::time::timeout(
                Duration::from_millis(200),
                tokio::task::spawn_blocking(|| 7usize),
            )
            .await
            .expect("blocking pool sentinel must run promptly")
            .expect("blocking sentinel must not panic");

            assert_eq!(sentinel, 7);
        });
    }

    /// A guest whose initial memory exceeds the cap must be denied, and the
    /// denial must classify as a resource limit.
    #[test]
    fn oversized_guest_is_denied_by_memory_cap() {
        let engine = Engine::default();
        let module = module_from_wat(&engine, BIG_MEM_WAT, "compile big-mem guest");
        let mut store = Store::new(&engine, HostLimits::new(Some(1024 * 1024)));
        store.limiter(|limits: &mut HostLimits| limits);
        let linker: Linker<HostLimits> = Linker::new(&engine);

        let error = linker
            .instantiate(&mut store, &module)
            .expect_err("initial memory over cap must be denied");
        let denied = store.data().memory_denied;
        assert!(denied, "limiter must record the denial");
        assert_eq!(
            error::classify_error(&error, denied).kind,
            error::FailureKind::ResourceLimit
        );
    }
}
