//! Native wasmtime hosts for Scryer's plugin runtimes.
//!
//! A process-wide engine with epoch cancellation and the full wasm feature
//! surface ([`engine`]), per-invocation sandboxes with a memory cap
//! ([`sandbox`]), the frozen crypto/CRC cores ([`crypto_host`]), the shared
//! host-call service layer every family world imports ([`command_host`]), the
//! WASI Preview 2 component hosts for indexers ([`component_host`]), archive
//! extractors ([`archive_component_host`]), subtitle providers
//! ([`subtitle_component_host`]), download clients
//! ([`download_client_component_host`]) and notification channels
//! ([`notification_component_host`]) — the last three sharing what
//! [`family_component_host`] holds — and trap→`AppError` mapping
//! ([`error`]). Everything else in the archive pipeline (path sandboxing,
//! native PAR2, providers, SDK shapes) is owned above this layer.

pub(crate) mod archive_component_host;
pub(crate) mod command_host;
pub(crate) mod component_host;
mod crypto_host;
pub(crate) mod download_client_component_host;
pub(crate) mod engine;
mod error;
mod family_component_host;
pub(crate) mod module_cache;
pub(crate) mod notification_component_host;
mod sandbox;
pub(crate) mod subtitle_component_host;

pub(crate) use archive_component_host::{
    ArchiveInvocation, archive_component_describe, process_archive_component,
    validate_archive_component,
};
pub(crate) use component_host::validate_indexer_component;
pub(crate) use download_client_component_host::{
    DownloadClientComponentInvocation, download_client_component_describe,
    process_download_client_component, validate_download_client_component,
};
pub(crate) use notification_component_host::{
    NotificationComponentInvocation, notification_component_describe,
    process_notification_component, validate_notification_component,
};
pub(crate) use subtitle_component_host::{
    SubtitleComponentInvocation, process_subtitle_component, subtitle_component_describe,
    validate_subtitle_component,
};
