//! Subtitle search, provider integration, scoring, sync, and orchestration.
//!
//! Start with `orchestration.rs` for the application-layer polling and trigger flow.
//! Provider behavior and language utilities live in the sibling files.

pub mod configs;
pub mod download;
mod external;
#[cfg(feature = "runtime-media-analysis")]
mod external_probe;
#[cfg(not(feature = "runtime-media-analysis"))]
#[path = "external_probe_stub.rs"]
mod external_probe;
#[cfg(feature = "runtime-archives")]
pub mod extraction;
#[cfg(not(feature = "runtime-archives"))]
#[path = "extraction_stub.rs"]
pub mod extraction;
pub mod language;
pub mod orchestration;
pub mod provider;
pub mod scoring;
pub mod search;
#[cfg(feature = "runtime-media-analysis")]
pub mod sync;
#[cfg(not(feature = "runtime-media-analysis"))]
#[path = "sync_stub.rs"]
pub mod sync;
pub mod wanted;

pub(crate) use external::{
    ExternalSubtitleDirectoryCache, reconcile_external_subtitles_for_media_file,
    reconcile_external_subtitles_for_media_file_with_cache,
};
pub use external_probe::{ExternalSubtitleDetectionSource, ExternalSubtitleProbeCacheEntry};
pub use language::{normalize_subtitle_language_code, same_subtitle_language};
pub use provider::{
    SubtitleCommunityEntry, SubtitleFile, SubtitleMatch, SubtitleMediaKind, SubtitleProvider,
    SubtitleQuery,
};
pub use scoring::{MovieScore, SeriesScore};
pub use search::SubtitleSearchOrchestrator;
