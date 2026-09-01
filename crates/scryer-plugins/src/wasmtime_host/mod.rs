//! Native wasmtime hosts for Scryer's plugin runtimes.
//!
//! A process-wide engine with epoch cancellation and the full wasm feature
//! surface ([`engine`]), per-invocation sandboxes with a memory cap
//! ([`sandbox`]), the frozen crypto/CRC cores ([`crypto_host`]), the
//! stdin/stdout command protocol for wasip1 command guests ([`invoke`]), the
//! WASI Preview 2 component hosts for indexers ([`component_host`]), archive
//! extractors ([`archive_component_host`]) and subtitle providers
//! ([`subtitle_component_host`]), and trap→`AppError` mapping
//! ([`error`]). Everything else in the archive pipeline (path sandboxing,
//! native PAR2, providers, SDK shapes) is owned above this layer.

pub(crate) mod archive_component_host;
pub(crate) mod command_host;
pub(crate) mod component_host;
mod crypto_host;
mod describe;
pub(crate) mod engine;
mod error;
mod invoke;
pub(crate) mod module_cache;
mod sandbox;
pub(crate) mod subtitle_component_host;

pub(crate) use archive_component_host::{
    ARCHIVE_CORE_MODULE_REJECTED, ArchiveInvocation, archive_component_describe,
    process_archive_component, validate_archive_component,
};
pub(crate) use component_host::validate_indexer_component;
pub(crate) use describe::{
    command_model_describe, validate_command_module, validate_subtitle_sync_module,
};
pub(crate) use invoke::{
    CommandInvocation, SubtitleSyncInvocation, process_command, process_subtitle_sync,
};
pub(crate) use subtitle_component_host::{
    SubtitleComponentInvocation, process_subtitle_component, subtitle_component_describe,
    validate_subtitle_component,
};
