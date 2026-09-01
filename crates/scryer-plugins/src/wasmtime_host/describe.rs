//! Descriptor extraction for command-model artifacts.
//!
//! Command plugins are wasip1 command binaries: they export `_start` and
//! `memory` but NOT `scryer_describe`. Scryer's
//! Extism describe path (`plugin.call("scryer_describe")`) and its
//! export-existence validation would therefore reject it. This module detects
//! the command shape and runs describe through the wasmtime backing instead;
//! legacy reactor plugins keep the compatibility describe path untouched.

use scryer_application::{AppError, AppResult};
use scryer_plugin_sdk::{EXPORT_DESCRIBE, PluginDescriptor};
use wasmtime::{ExternType, Linker, Module, Store};

use crate::wasmtime_host::sandbox::{self, BareSandbox, HostCtx, HostLimits};
use crate::wasmtime_host::{command_host, engine, error, module_cache};

/// Describe runs reuse the 10s describe budget of the Extism path.
const DESCRIBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub(crate) fn validate_subtitle_sync_module(wasm: &[u8]) -> Result<(), String> {
    validate_command_module(wasm, "subtitle sync")
}

/// Validate the common wasip1 command shape used by marker-selected artifacts.
///
/// The archive extractor no longer has a core-module form: it is a WASI
/// Preview 2 component validated by `validate_archive_component`, so the
/// crypto host ABI this function once registered for it is gone with it.
pub(crate) fn validate_command_module(wasm: &[u8], kind: &str) -> Result<(), String> {
    let engine = engine::shared_async_engine();
    let module = module_cache::command_module(wasm)
        .map_err(|error| format!("failed to compile {kind} plugin WASM: {error}"))?;

    let mut linker: Linker<crate::wasmtime_host::sandbox::HostCtx> = Linker::new(engine);
    wasmtime_wasi::p1::add_to_linker_async(&mut linker, |ctx| &mut ctx.wasi)
        .map_err(|error| format!("failed to wire WASI preview1 for {kind} plugin: {error:#}"))?;
    command_host::add_to_linker(&mut linker)
        .map_err(|error| format!("failed to register native command host functions: {error:#}"))?;
    linker
        .instantiate_pre(&module)
        .map_err(|error| format!("{kind} plugin imports do not match the host ABI: {error:#}"))?;

    let start = module
        .exports()
        .find(|export| export.name() == "_start")
        .map(|export| export.ty());
    match start {
        Some(ExternType::Func(ty))
            if ty.params().next().is_none() && ty.results().next().is_none() => {}
        Some(ExternType::Func(_)) => {
            return Err("command plugin '_start' export must have type () -> ()".to_string());
        }
        _ => return Err("command plugin must export a function named '_start'".to_string()),
    }
    if !module
        .exports()
        .any(|export| export.name() == "memory" && matches!(export.ty(), ExternType::Memory(_)))
    {
        return Err("command plugin must export a linear memory named 'memory'".to_string());
    }
    Ok(())
}

/// Attempt to extract a descriptor from a command-model artifact.
///
/// Returns `None` when the artifact is NOT the command model, so the caller
/// falls back to the Extism describe path. Classification: the module
/// exports `_start` and does NOT export `scryer_describe`. `Some(Err(_))` means
/// it is the command model but describe failed (e.g. missing `memory` export or
/// a bad describe response).
pub(crate) fn command_model_describe(wasm: &[u8]) -> Option<Result<PluginDescriptor, String>> {
    // Cheap negative fast-path: every Extism fleet plugin exports
    // `scryer_describe`, whose name is present verbatim in the wasm export
    // section, so skip the wasmtime compile for those. The authoritative
    // classification below still uses wasmtime module exports for the command
    // case where it matters.
    if contains_bytes(wasm, EXPORT_DESCRIBE.as_bytes()) {
        return None;
    }

    // If it will not even compile under wasmtime, we cannot classify it here;
    // let the Extism path report its own error.
    let module = module_cache::legacy_module(wasm).ok()?;

    let mut has_start = false;
    let mut has_describe = false;
    let mut has_memory = false;
    for export in module.exports() {
        match export.name() {
            "_start" => has_start = true,
            name if name == EXPORT_DESCRIBE => has_describe = true,
            "memory" => has_memory = matches!(export.ty(), ExternType::Memory(_)),
            _ => {}
        }
    }

    if !has_start || has_describe {
        // Extism reactor model (or not a command) — fall back.
        return None;
    }
    if !has_memory {
        return Some(Err(
            "command plugin must export a linear memory named 'memory'".to_string(),
        ));
    }

    Some(run_describe(&module).map_err(|error| error.to_string()))
}

fn run_describe(module: &Module) -> AppResult<PluginDescriptor> {
    let engine = engine::shared_engine();

    let mut linker: Linker<HostCtx> = Linker::new(engine);
    wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |ctx: &mut HostCtx| &mut ctx.wasi).map_err(
        |error| {
            AppError::Repository(format!(
                "failed to wire WASI for command plugin describe: {error:#}"
            ))
        },
    )?;
    command_host::add_to_linker(&mut linker).map_err(|error| {
        AppError::Repository(format!(
            "failed to register native command host functions for describe: {error:#}"
        ))
    })?;
    let BareSandbox {
        wasi,
        stdout,
        stderr,
    } = sandbox::build_describe_sandbox();

    let mut store = Store::new(engine, HostCtx::new(wasi, HostLimits::new(None)));
    store.limiter(|ctx: &mut HostCtx| &mut ctx.limits);
    store.set_epoch_deadline(engine::deadline_ticks(DESCRIBE_TIMEOUT));

    let instance = linker.instantiate(&mut store, module).map_err(|error| {
        AppError::Repository(format!(
            "failed to instantiate command plugin for describe: {error:#}"
        ))
    })?;
    let start = instance
        .get_typed_func::<(), ()>(&mut store, "_start")
        .map_err(|error| {
            AppError::Repository(format!("command plugin is not a wasip1 command: {error:#}"))
        })?;

    let result = start.call(&mut store, ());
    let denied = store.data().limits.memory_denied;
    let stdout_bytes = stdout.contents();
    let stderr_bytes = stderr.contents();
    let stderr_tail = {
        let start = stderr_bytes.len().saturating_sub(4096);
        String::from_utf8_lossy(&stderr_bytes[start..]).into_owned()
    };

    error::interpret_start_result(result, denied).map_err(|failure| {
        let ctx = error::InvocationContext {
            plugin_id: "<command-describe>",
            plugin_version: "",
            operation: "describe",
            budget: DESCRIBE_TIMEOUT,
            stderr_tail: &stderr_tail,
        };
        error::to_app_error(&failure, &ctx)
    })?;

    serde_json::from_slice::<PluginDescriptor>(&stdout_bytes).map_err(|error| {
        AppError::Repository(format!(
            "command plugin describe returned invalid PluginDescriptor JSON: {error}"
        ))
    })
}

/// Naive substring search — good enough for a cheap negative pre-filter.
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.len() < needle.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_command_validation_uses_exact_host_abi() {
        let valid = wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "_start")))"#,
        )
        .unwrap();
        validate_subtitle_sync_module(&valid).expect("valid subtitle command");

        let native_host_imports = wat::parse_str(
            r#"(module
                (import "scryer:host/v1" "scryer_host_call" (func (param i32 i32) (result i32)))
                (import "scryer:host/v1" "scryer_host_response_len" (func (param i32) (result i32)))
                (import "scryer:host/v1" "scryer_host_response_read" (func (param i32 i32 i32) (result i32)))
                (import "scryer:host/v1" "scryer_host_response_drop" (func (param i32)))
                (memory (export "memory") 1)
                (func (export "_start")))"#,
        )
        .unwrap();
        validate_command_module(&native_host_imports, "native host imports")
            .expect("native command host imports are supported");

        let unknown_import = wat::parse_str(
            r#"(module
                (import "wasi_snapshot_preview1" "not_a_host_function" (func))
                (memory (export "memory") 1)
                (func (export "_start")))"#,
        )
        .unwrap();
        let error = validate_subtitle_sync_module(&unknown_import)
            .expect_err("unknown host import must fail");
        assert!(error.contains("imports do not match"), "{error}");

        let wrong_start = wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "_start") (param i32)))"#,
        )
        .unwrap();
        let error = validate_subtitle_sync_module(&wrong_start)
            .expect_err("wrong _start signature must fail");
        assert!(error.contains("() -> ()"), "{error}");
    }

    #[test]
    fn embedded_command_validation_uses_host_engine_features() {
        let threads = wat::parse_str(
            r#"(module
                (memory (export "memory") 1 1 shared)
                (func (export "_start")))"#,
        )
        .unwrap();
        let error = validate_subtitle_sync_module(&threads)
            .expect_err("threads are disabled by the host engine");
        assert!(error.contains("compile"), "{error}");
    }

    #[test]
    fn extism_reactor_shape_falls_back_to_none() {
        // A module exporting `scryer_describe` is the Extism reactor model — the
        // negative fast-path (substring on the export name) returns None so the
        // caller uses the Extism describe path.
        let wasm = wat::parse_str(
            r#"(module
                 (func (export "scryer_describe") (result i64) (i64.const 0))
                 (memory (export "memory") 1))"#,
        )
        .unwrap();
        assert!(command_model_describe(&wasm).is_none());
    }

    #[test]
    fn command_shape_without_memory_is_rejected() {
        // Exports `_start`, no `scryer_describe`, but no `memory` -> classified
        // as command-model and rejected for the missing memory export.
        let wasm = wat::parse_str(r#"(module (func (export "_start")))"#).unwrap();
        match command_model_describe(&wasm) {
            Some(Err(message)) => assert!(message.contains("memory"), "{message}"),
            other => panic!("expected Some(Err(missing memory)), got {other:?}"),
        }
    }

    #[test]
    fn non_command_module_falls_back_to_none() {
        // No `_start`, no `scryer_describe` -> not a command; fall back to None.
        let wasm = wat::parse_str(r#"(module (memory (export "memory") 1))"#).unwrap();
        assert!(command_model_describe(&wasm).is_none());
    }

    #[test]
    fn contains_bytes_matches() {
        assert!(contains_bytes(b"abcdef", b"cde"));
        assert!(!contains_bytes(b"abc", b"xyz"));
        assert!(contains_bytes(b"anything", b""));
        assert!(!contains_bytes(b"ab", b"abc"));
    }
}
