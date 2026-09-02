use super::*;
use crate::event_views::compare_download_queue_items;
use crate::services::DownloadQueueSync;
use crate::tracked_downloads::{
    TrackedDownload, TrackedDownloadQueueMetadata, publish_runtime_tracked_download_snapshot_cache,
    tracked_download_id_for_item,
};
use crate::types::{DownloadClientFilterOption, DownloadQueuePage};
use futures_util::FutureExt;
use scryer_domain::{
    CompletedDownload, DownloadQueueDeleteStatus, ImportType, TrackedDownloadState,
    TrackedDownloadStatus,
};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

// This facade keeps the previous module scope while the former junk drawer is
// mechanically split into functional source files.
include!("indexers.rs");
include!("proxies.rs");
include!("seeding_profiles.rs");
include!("managed_indexers.rs");
include!("download_clients.rs");
include!("queue_projection.rs");
include!("queue_queries.rs");
include!("manual_import_sources.rs");
include!("tracked_commands.rs");
include!("queue_mutations.rs");
include!("subscriptions.rs");
include!("permissions.rs");
include!("tests.rs");
