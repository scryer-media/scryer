use super::*;
use crate::acquisition::acquisition::submission_blocks_wanted_item;
use crate::acquisition::submission::{
    CanonicalDownloadSubmissionIntent, CanonicalDownloadSubmissionOutcome,
};
use crate::acquisition_decision_helpers::is_download_submit_unavailable_error;
use crate::catalog_helpers::{
    DownloadClientRoutingEntry, anime_mapping_identity_keys, anime_movie_after_season,
    anime_movie_identity_keys, anime_movie_release_sort_key, build_rematched_external_ids,
    default_download_client_routing_entry, is_logical_specials_collection,
    movie_entity_from_anime_movie, parse_download_client_routing_entry,
    parse_download_client_routing_map, release_is_recent_for_queue_priority,
    series_movie_link_from_anime_movie, strip_derived_match_tags,
};
use crate::contracts::{
    QueueDownloadOutcome, QueuedDownloadResult, SubmissionConflictPolicy, SubmissionScopeConflict,
};
use crate::domain_events::{
    DomainEventActor, deleted_media_update, new_title_domain_event, title_context_snapshot,
};
use crate::settings::settings::root_folder_entries_from_library_roots;
use scryer_domain::{
    DomainEventPayload, JobRunCompletedEventData, JobRunFailedEventData, JobRunStartedEventData,
    MediaFileDeletedEventData, MediaFileDeletedReason, MetadataHydrationState,
    ReleaseGrabbedEventData, SeriesMovieLink, TitleAddedEventData, TitleDeletedEventData,
    TitleRematchedEventData,
};
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

// This facade keeps the previous module scope while the former junk drawer is
// mechanically split into functional source files.
include!("roots.rs");
include!("titles.rs");
include!("hydration.rs");
include!("queueing.rs");
include!("monitoring.rs");
include!("delete.rs");
include!("metadata.rs");
include!("collections.rs");
include!("permissions.rs");
include!("tests.rs");
