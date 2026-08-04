//! Native wasmtime host for the archive-extractor plugin.
//!
//! Replaces the Extism execution path for the archive kind: a process-wide
//! engine with epoch cancellation and the full wasm feature surface
//! ([`engine`]), a per-invocation WASI p1 sandbox with a memory cap
//! ([`sandbox`]), the frozen zero-copy crypto/CRC host ABI ([`crypto_host`]),
//! the stdin/stdout command protocol ([`invoke`]), and trap→`AppError` mapping
//! ([`error`]). Everything else in the archive pipeline (path sandboxing,
//! native PAR2, providers, SDK shapes) is owned above this layer.

pub(crate) mod command_host;
mod crypto_host;
mod describe;
pub(crate) mod engine;
mod error;
mod invoke;
pub(crate) mod module_cache;
mod sandbox;

pub(crate) use describe::{
    command_model_describe, validate_archive_module, validate_command_module,
    validate_subtitle_sync_module,
};
pub(crate) use invoke::{
    ArchiveInvocation, CommandInvocation, SubtitleSyncInvocation, process_archive, process_command,
    process_subtitle_sync,
};
