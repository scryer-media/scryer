pub const TEST_CONNECTION_QUERY: &str = include_str!("weaver/test_connection.graphql");
pub const VERSION_COMPAT_QUERY: &str = include_str!("weaver/version_compat.graphql");
pub const QUEUE_ITEMS_QUERY: &str = include_str!("weaver/queue_items.graphql");
pub const HISTORY_ITEM_QUERY: &str = include_str!("weaver/history_item.graphql");
pub const HISTORY_ITEMS_QUERY: &str = include_str!("weaver/history_items.graphql");
// Asked only for a download whose file names look cut. TODO(2026-11-17): select
// `postedFileNames` in the history reads themselves and drop this query, once
// Weavers that refuse the field are rare.
pub const HISTORY_ITEM_POSTED_FILE_NAMES_QUERY: &str =
    include_str!("weaver/history_item_posted_file_names.graphql");
pub const JOBS_COMPAT_QUERY: &str = include_str!("weaver/jobs_compat.graphql");
pub const SUBMIT_NZB_MUTATION: &str = include_str!("weaver/submit_nzb.graphql");
pub const SUBMIT_NZB_COMPAT_MUTATION: &str = include_str!("weaver/submit_nzb_compat.graphql");
pub const PAUSE_QUEUE_ITEM_MUTATION: &str = include_str!("weaver/pause_queue_item.graphql");
pub const PAUSE_JOB_MUTATION: &str = include_str!("weaver/pause_job.graphql");
pub const RESUME_QUEUE_ITEM_MUTATION: &str = include_str!("weaver/resume_queue_item.graphql");
pub const RESUME_JOB_MUTATION: &str = include_str!("weaver/resume_job.graphql");
pub const REMOVE_HISTORY_ITEMS_MUTATION: &str = include_str!("weaver/remove_history_items.graphql");
pub const REMOVE_HISTORY_ITEMS_DELETE_FILES_MUTATION: &str =
    include_str!("weaver/remove_history_items_delete_files.graphql");
pub const DELETE_HISTORY_BATCH_MUTATION: &str = include_str!("weaver/delete_history_batch.graphql");
pub const CANCEL_QUEUE_ITEM_MUTATION: &str = include_str!("weaver/cancel_queue_item.graphql");
pub const CANCEL_JOB_MUTATION: &str = include_str!("weaver/cancel_job.graphql");
pub const QUEUE_ITEMS_WITH_TORRENTS_QUERY: &str =
    include_str!("weaver/queue_items_torrents.graphql");
pub const QUEUE_ITEM_PROTOCOL_QUERY: &str = include_str!("weaver/queue_item_protocol.graphql");
pub const HISTORY_ITEMS_WITH_PROTOCOL_QUERY: &str =
    include_str!("weaver/history_items_protocol.graphql");
pub const HISTORY_ITEM_WITH_PROTOCOL_QUERY: &str =
    include_str!("weaver/history_item_protocol.graphql");
pub const TORRENT_SUPPORT_QUERY: &str = include_str!("weaver/torrent_support.graphql");
pub const SUBMIT_TORRENT_MUTATION: &str = include_str!("weaver/submit_torrent.graphql");
pub const REMOVE_TORRENT_MUTATION: &str = include_str!("weaver/remove_torrent.graphql");
pub const QUEUE_SNAPSHOTS_WITH_TORRENTS_SUBSCRIPTION: &str =
    include_str!("weaver/queue_snapshots_torrents.graphql");
pub const QUEUE_EVENTS_WITH_TORRENTS_SUBSCRIPTION: &str =
    include_str!("weaver/queue_events_torrents.graphql");
pub const TORRENT_ROW_UPDATES_SUBSCRIPTION: &str =
    include_str!("weaver/torrent_row_updates.graphql");
