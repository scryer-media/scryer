use super::{ActorKindValue, Long, MediaFacetValue};
use async_graphql::{Enum, ID, InputObject, Json, SimpleObject};
use chrono::{DateTime, Utc};
use scryer_domain::TitleHistoryEventType;

// ── Subtitle downloads ──────────────────────────────────────────────────────

#[derive(async_graphql::SimpleObject)]
/// Subtitle file downloaded for a media file.
pub struct ExternalSubtitlePayload {
    /// Subtitle record ID.
    pub id: ID,
    /// Media file ID containing the subtitle.
    pub media_file_id: ID,
    /// Title ID owning the media file.
    pub title_id: ID,
    /// Episode ID, or null for movie or series-level subtitles.
    pub episode_id: Option<ID>,
    /// Subtitle source category.
    pub source_kind: String,
    /// Subtitle language code.
    pub language: String,
    /// Subtitle provider, or null when provider metadata is unavailable.
    pub provider: Option<String>,
    /// Provider subtitle file ID, or null when unavailable.
    pub provider_file_id: Option<String>,
    /// Local subtitle file path.
    pub file_path: String,
    /// Provider score, when supplied.
    pub score: Option<i32>,
    /// Provider score as a percentage, when supplied.
    pub score_percent: Option<i32>,
    /// Whether the subtitle is marked for hearing-impaired viewers.
    pub hearing_impaired: bool,
    /// Whether the subtitle is forced.
    pub forced: bool,
    /// Whether the subtitle was translated by an AI system.
    pub ai_translated: bool,
    /// Whether the subtitle was translated by a machine process.
    pub machine_translated: bool,
    /// Provider uploader, or null when unavailable.
    pub uploader: Option<String>,
    /// Provider release information, or null when unavailable.
    pub release_info: Option<String>,
    /// Whether the subtitle timing is synchronized.
    pub synced: bool,
    /// Download time in UTC.
    pub downloaded_at: DateTime<Utc>,
}

#[derive(async_graphql::SimpleObject)]
/// Subtitle provider record blocked for a media file.
pub struct ExternalSubtitleBlocklistEntryPayload {
    /// Blocklist entry ID.
    pub id: ID,
    /// Media file ID affected by the blocklist entry.
    pub media_file_id: ID,
    /// Subtitle provider.
    pub provider: String,
    /// Provider subtitle file ID.
    pub provider_file_id: String,
    /// Blocked subtitle language code.
    pub language: String,
    /// Blocklist reason, or null when none was recorded.
    pub reason: Option<String>,
    /// Creation time in UTC.
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Title History
// ---------------------------------------------------------------------------

#[derive(SimpleObject, Clone)]
/// One event in a title's acquisition and file history.
pub struct TitleHistoryEventPayload {
    /// History event ID.
    pub id: ID,
    /// Title ID associated with the event.
    pub title_id: ID,
    /// Title name, or null when no name was available.
    pub title_name: Option<String>,
    /// Poster URL of the title, or null when the title is gone or has no poster.
    pub poster_url: Option<String>,
    /// Library owning the title, or null when the event cannot be attributed.
    pub library_id: Option<ID>,
    /// Media facet, or null when the event is not facet-specific.
    pub facet: Option<MediaFacetValue>,
    /// Bytes imported or upgraded; null for other event types and for events recorded before sizes were captured.
    pub size_bytes: Option<Long>,
    /// Episode ID, or null when not episode-specific.
    pub episode_id: Option<ID>,
    /// Episode IDs affected by the event.
    pub episode_ids: Vec<ID>,
    /// Collection ID, or null when not collection-specific.
    pub collection_id: Option<ID>,
    /// Stable domain event code represented by this history row.
    pub event_type: String,
    /// Kind of actor that caused the event, or null when unknown.
    pub actor_kind: Option<ActorKindValue>,
    /// Acting user ID, or null for non-user actors.
    pub actor_user_id: Option<ID>,
    /// Acting user display name, or null when unavailable.
    pub actor_display_name: Option<String>,
    /// Title as supplied by the source, or null when unavailable.
    pub source_title: Option<String>,
    /// Display title recorded for the event, or null when unavailable.
    pub display_title: Option<String>,
    /// Source system name, or null when unavailable.
    pub source_system: Option<String>,
    /// Source reference, or null when unavailable.
    pub source_ref: Option<String>,
    /// Source provider, or null when unavailable.
    pub source_provider: Option<String>,
    /// Download source locator associated with the event, or null when unavailable.
    pub source_hint: Option<String>,
    /// Quality label, or null when unavailable.
    pub quality: Option<String>,
    /// Download identity, or null when unavailable.
    pub download_id: Option<String>,
    /// Download client ID, or null when unavailable.
    pub client_id: Option<ID>,
    /// Download client name, or null when unavailable.
    pub client_name: Option<String>,
    /// Import job ID, or null when unavailable.
    pub import_id: Option<ID>,
    /// Skip reason, or null when the event was not skipped.
    pub skip_reason: Option<String>,
    /// Whether retrying requires a password.
    pub retry_requires_password: bool,
    /// Failure reason, or null when the event did not fail.
    pub failure_reason: Option<String>,
    /// Blocklist reason, or null when the event was not blocklisted.
    pub blocklist_reason: Option<String>,
    /// Source file path, or null when unavailable.
    pub source_path: Option<String>,
    /// Destination file path, or null when unavailable.
    pub dest_path: Option<String>,
    /// Additional event data as JSON, or null when absent.
    pub data_json: Option<Json<serde_json::Value>>,
    /// Time the event occurred in UTC.
    pub occurred_at: DateTime<Utc>,
    /// Time the event record was created in UTC.
    pub created_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
/// A page of title history events and pagination state.
pub struct TitleHistoryPagePayload {
    /// Events in the requested page.
    pub items: Vec<TitleHistoryEventPayload>,
    /// Total matching events across all pages.
    pub total_count: i64,
    /// Whether more matching events exist after this page.
    pub has_more: bool,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
/// Event types available to title history filters.
pub enum TitleHistoryEventTypeValue {
    /// Title request was created.
    Requested,
    /// Release was accepted for grabbing.
    Grabbed,
    /// Download failed.
    DownloadFailed,
    /// Release was blocklisted.
    Blocklisted,
    /// Download completed.
    DownloadCompleted,
    /// File was imported.
    Imported,
    /// Import failed.
    ImportFailed,
    /// Import was skipped.
    ImportSkipped,
    /// Existing file was upgraded.
    FileUpgraded,
    /// File was moved to the recycle bin.
    FileRecycled,
    /// File was deleted.
    FileDeleted,
    /// File was renamed.
    FileRenamed,
    /// Title completed a root move or library transfer.
    TitleMoved,
    /// Download was ignored.
    DownloadIgnored,
    /// Title or file was rematched.
    Rematched,
    /// Torrent was imported and its client entry retained while it seeds.
    SeedingStarted,
    /// Torrent's seeding obligation was discharged.
    SeedingCompleted,
}

impl TitleHistoryEventTypeValue {
    pub fn into_domain(self) -> TitleHistoryEventType {
        match self {
            Self::Requested => TitleHistoryEventType::Requested,
            Self::Grabbed => TitleHistoryEventType::Grabbed,
            Self::DownloadFailed => TitleHistoryEventType::DownloadFailed,
            Self::Blocklisted => TitleHistoryEventType::Blocklisted,
            Self::DownloadCompleted => TitleHistoryEventType::DownloadCompleted,
            Self::Imported => TitleHistoryEventType::Imported,
            Self::ImportFailed => TitleHistoryEventType::ImportFailed,
            Self::ImportSkipped => TitleHistoryEventType::ImportSkipped,
            Self::FileUpgraded => TitleHistoryEventType::FileUpgraded,
            Self::FileRecycled => TitleHistoryEventType::FileRecycled,
            Self::FileDeleted => TitleHistoryEventType::FileDeleted,
            Self::FileRenamed => TitleHistoryEventType::FileRenamed,
            Self::TitleMoved => TitleHistoryEventType::TitleMoved,
            Self::DownloadIgnored => TitleHistoryEventType::DownloadIgnored,
            Self::Rematched => TitleHistoryEventType::Rematched,
            Self::SeedingStarted => TitleHistoryEventType::SeedingStarted,
            Self::SeedingCompleted => TitleHistoryEventType::SeedingCompleted,
        }
    }
}

#[derive(InputObject)]
/// Filters and pagination controls for title history.
pub struct TitleHistoryFilterInput {
    /// Event types to include, or null for all types.
    pub event_types: Option<Vec<TitleHistoryEventTypeValue>>,
    /// Title IDs to include, or null for all titles.
    pub title_ids: Option<Vec<ID>>,
    /// Library IDs to include, or null for all libraries.
    pub library_ids: Option<Vec<ID>>,
    /// Case-insensitive title search text, or null for no text filter.
    pub title_search: Option<String>,
    /// Download identity to include, or null for all downloads.
    pub download_id: Option<String>,
    /// Episode ID to include, or null for all episodes.
    pub episode_id: Option<ID>,
    /// Whether equivalent events should be grouped.
    pub group_by_event: Option<bool>,
    /// Maximum number of events to return.
    pub limit: Option<i32>,
    /// Number of matching events to skip.
    pub offset: Option<i32>,
}
