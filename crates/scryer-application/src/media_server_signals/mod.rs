//! Media-server watch signals (RFC 137 sections 4.4, 6.4, 7.3).
//!
//! A provider-neutral store of *who watched what*, kept alongside
//! [`crate::media_servers`] rather than inside it: connections are
//! configuration, signals are observations, and only the observations are
//! policy input.
//!
//! # Shape
//!
//! * [`sync`] — the scheduled sweep: connections → participants → adapter read
//!   → mapping → atomic per-participant replace → per-connection state.
//! * [`mapping`] — pure provider-item → Scryer-subject resolution.
//!
//! The provider HTTP details are not here at all. They live behind
//! [`crate::ports::MediaServerSignalSource`], which is implemented once in
//! infrastructure and dispatches on the connection's provider, so this module
//! never learns what a Jellyfin URL looks like.
//!
//! # Scope of this wave
//!
//! Jellyfin only, movies and episodes only, no show-level rollups, and no
//! consent surface: played state is read with the connection's stored admin
//! key, which the RFC classifies as `server_admin` visibility. Emby and Plex
//! are adapter arms, not redesigns.
//!
//! # What is not here
//!
//! No maintenance-rule facts and no GraphQL. The signals are collected and
//! stored here; the fact-snapshot builder that turns them into rule facts lives
//! in [`crate::maintenance_rules::facts`] and reaches them only through the two
//! batched, title-keyed reads on
//! [`crate::ports::MediaServerSignalRepository`].

pub mod mapping;
pub mod sync;

pub use mapping::{
    EpisodeNumberIndex, MappedSignalSubject, SIGNAL_EXTERNAL_ID_SOURCES, TitleExternalIdIndex,
    resolve_episode, resolve_subject, resolve_title,
};
pub use sync::{
    MediaServerSignalSyncReport, SIGNAL_SYNC_PROVIDERS, log_signal_sync_report, new_signal_id,
    signal_sync_summary,
};
