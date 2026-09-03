use async_graphql::{Enum, InputValueError, InputValueResult, Scalar, ScalarType, Value};
use chrono::NaiveDate;
use scryer_domain::{
    AppPermission, CollectionType, DomainEventActorKind, DomainEventType, DownloadQueueState,
    EpisodeType, ExecutionMode, ImportDecision, ImportErrorCode, ImportMode, ImportSkipReason,
    ImportStatus, ImportTransferPhase, ImportType, LibraryPermission, MediaFacet,
    MediaRequestStatus, TitleMatchType, TrackedDownloadState, TrackedDownloadStatus,
};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Long(pub i64);

impl Long {
    pub fn from_u64_saturating(value: u64) -> Self {
        Self(i64::try_from(value).unwrap_or(i64::MAX))
    }
}

impl From<i64> for Long {
    fn from(value: i64) -> Self {
        Self(value)
    }
}

impl From<Long> for i64 {
    fn from(value: Long) -> Self {
        value.0
    }
}

/// Signed 64-bit integer scalar for counts, sizes, sequences, and other values that exceed GraphQL `Int` range.
#[Scalar(name = "Long")]
impl ScalarType for Long {
    fn parse(value: Value) -> InputValueResult<Self> {
        match value {
            Value::Number(number) => number
                .as_i64()
                .map(Self)
                .ok_or_else(|| InputValueError::custom("Long must be a signed 64-bit integer")),
            other => Err(InputValueError::expected_type(other)),
        }
    }

    fn is_valid(value: &Value) -> bool {
        matches!(value, Value::Number(number) if number.as_i64().is_some())
    }

    fn to_value(&self) -> Value {
        Value::Number(self.0.into())
    }
}

/// Calendar date scalar serialized as an ISO-8601 `YYYY-MM-DD` string without a time zone.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Date(pub NaiveDate);

impl Date {
    pub fn parse_iso(value: &str) -> Result<Self, chrono::ParseError> {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").map(Self)
    }

    pub fn to_iso_string(self) -> String {
        self.0.format("%Y-%m-%d").to_string()
    }
}

impl From<NaiveDate> for Date {
    fn from(value: NaiveDate) -> Self {
        Self(value)
    }
}

impl From<Date> for NaiveDate {
    fn from(value: Date) -> Self {
        value.0
    }
}

/// Calendar date scalar serialized as an ISO-8601 `YYYY-MM-DD` string without a time zone.
#[Scalar(name = "Date")]
impl ScalarType for Date {
    fn parse(value: Value) -> InputValueResult<Self> {
        match value {
            Value::String(value) => {
                Self::parse_iso(&value).map_err(|error| InputValueError::custom(error.to_string()))
            }
            other => Err(InputValueError::expected_type(other)),
        }
    }

    fn is_valid(value: &Value) -> bool {
        matches!(value, Value::String(value) if Self::parse_iso(value).is_ok())
    }

    fn to_value(&self) -> Value {
        Value::String(self.to_iso_string())
    }
}

/// Media facet used to distinguish movie, series, and anime catalog records.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MediaFacetValue {
    /// Movie content.
    Movie,
    /// Series content.
    Series,
    /// Anime content.
    Anime,
}

/// Library permission granted within a specific library.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LibraryPermissionValue {
    /// Allows viewing library content.
    View,
    /// Allows managing titles in the library.
    ManageTitles,
    /// Allows resolving imports for the library.
    ResolveImports,
    /// Allows changing library configuration.
    ManageLibrary,
    /// Allows submitting requests for the library.
    Request,
    /// Allows automatically approving requests for the library.
    AutoApproveRequests,
}

/// Application-wide permission independent of a library.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum AppPermissionValue {
    /// Allows creating, changing, and deleting users.
    ManageUsers,
    /// Allows changing user permission grants.
    ManagePermissions,
    /// Allows changing system settings.
    ManageSystemSettings,
    /// Allows changing catalog settings.
    ManageCatalogSettings,
}

/// Account origin used for login and authorization behavior.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum UserAccountKindValue {
    /// Account created and managed locally.
    Local,
    /// Account created automatically from an external provider login.
    ExternalAutoProvisioned,
}

impl UserAccountKindValue {
    pub fn from_domain(kind: scryer_domain::UserAccountKind) -> Self {
        match kind {
            scryer_domain::UserAccountKind::Local => Self::Local,
            scryer_domain::UserAccountKind::ExternalAutoProvisioned => {
                Self::ExternalAutoProvisioned
            }
        }
    }
}

impl AppPermissionValue {
    pub fn into_domain(self) -> AppPermission {
        match self {
            Self::ManageUsers => AppPermission::ManageUsers,
            Self::ManagePermissions => AppPermission::ManagePermissions,
            Self::ManageSystemSettings => AppPermission::ManageSystemSettings,
            Self::ManageCatalogSettings => AppPermission::ManageCatalogSettings,
        }
    }

    pub fn from_domain(permission: AppPermission) -> Self {
        match permission {
            AppPermission::ManageUsers => Self::ManageUsers,
            AppPermission::ManagePermissions => Self::ManagePermissions,
            AppPermission::ManageSystemSettings => Self::ManageSystemSettings,
            AppPermission::ManageCatalogSettings => Self::ManageCatalogSettings,
        }
    }
}

impl LibraryPermissionValue {
    pub fn into_domain(self) -> LibraryPermission {
        match self {
            Self::View => LibraryPermission::View,
            Self::ManageTitles => LibraryPermission::ManageTitles,
            Self::ResolveImports => LibraryPermission::ResolveImports,
            Self::ManageLibrary => LibraryPermission::ManageLibrary,
            Self::Request => LibraryPermission::Request,
            Self::AutoApproveRequests => LibraryPermission::AutoApproveRequests,
        }
    }

    pub fn from_domain(permission: LibraryPermission) -> Self {
        match permission {
            LibraryPermission::View => Self::View,
            LibraryPermission::ManageTitles => Self::ManageTitles,
            LibraryPermission::ResolveImports => Self::ResolveImports,
            LibraryPermission::ManageLibrary => Self::ManageLibrary,
            LibraryPermission::Request => Self::Request,
            LibraryPermission::AutoApproveRequests => Self::AutoApproveRequests,
        }
    }
}

impl MediaFacetValue {
    pub fn as_scope_id(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Series => "series",
            Self::Anime => "anime",
        }
    }

    pub fn into_domain(self) -> MediaFacet {
        match self {
            Self::Movie => MediaFacet::Movie,
            Self::Series => MediaFacet::Series,
            Self::Anime => MediaFacet::Anime,
        }
    }

    pub fn from_domain(value: MediaFacet) -> Self {
        match value {
            MediaFacet::Movie => Self::Movie,
            MediaFacet::Series => Self::Series,
            MediaFacet::Anime => Self::Anime,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "movie" => Some(Self::Movie),
            "series" => Some(Self::Series),
            "anime" => Some(Self::Anime),
            _ => None,
        }
    }
}

/// State of a pending import record.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum PendingImportStatusValue {
    /// Import is awaiting a decision or action.
    Pending,
    /// Import was intentionally ignored.
    Ignored,
}

/// Lifecycle state of a media request.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MediaRequestStatusValue {
    /// Request awaits a decision.
    Pending,
    /// Request was approved.
    Approved,
    /// Request was rejected.
    Rejected,
    /// Request was canceled.
    Canceled,
}

impl MediaRequestStatusValue {
    pub fn from_domain(value: MediaRequestStatus) -> Self {
        match value {
            MediaRequestStatus::Pending => Self::Pending,
            MediaRequestStatus::Approved => Self::Approved,
            MediaRequestStatus::Rejected => Self::Rejected,
            MediaRequestStatus::Canceled => Self::Canceled,
        }
    }

    pub fn into_domain(self) -> MediaRequestStatus {
        match self {
            Self::Pending => MediaRequestStatus::Pending,
            Self::Approved => MediaRequestStatus::Approved,
            Self::Rejected => MediaRequestStatus::Rejected,
            Self::Canceled => MediaRequestStatus::Canceled,
        }
    }
}

/// Content scope used by settings and catalog operations.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ContentScopeValue {
    /// Movie scope.
    Movie,
    /// Series scope.
    Series,
    /// Anime scope.
    Anime,
}

impl ContentScopeValue {
    pub fn as_scope_id(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Series => "series",
            Self::Anime => "anime",
        }
    }

    pub fn into_media_facet(self) -> MediaFacet {
        match self {
            Self::Movie => MediaFacet::Movie,
            Self::Series => MediaFacet::Series,
            Self::Anime => MediaFacet::Anime,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "movie" => Some(Self::Movie),
            "series" => Some(Self::Series),
            "anime" => Some(Self::Anime),
            _ => None,
        }
    }
}

/// Scoring persona that selects a quality-scoring strategy.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ScoringPersonaValue {
    /// Balanced scoring across quality and compatibility.
    Balanced,
    /// Favors audio quality.
    Audiophile,
    /// Favors efficient storage or delivery.
    Efficient,
    /// Favors broad playback compatibility.
    Compatible,
}

/// Monitoring mode applied to episodic content.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MonitorTypeValue {
    /// Monitor all selected episodes.
    Monitored,
    /// Monitor no episodes.
    Unmonitored,
    /// Monitor future episodes only.
    FutureEpisodes,
    /// Monitor missing and future episodes.
    MissingAndFutureEpisodes,
    /// Monitor every episode, including already released episodes.
    AllEpisodes,
    /// Monitor exactly the seasons and canon series movies the user selected.
    Advanced,
    /// Explicitly select no episodes; exposed as `NONE` in GraphQL.
    #[graphql(name = "NONE")]
    NoneSelected,
}

impl MonitorTypeValue {
    pub fn as_tag_value(self) -> &'static str {
        match self {
            Self::Monitored => "monitored",
            Self::Unmonitored => "unmonitored",
            Self::FutureEpisodes => "futureepisodes",
            Self::MissingAndFutureEpisodes => "missingandfutureepisodes",
            Self::AllEpisodes => "allepisodes",
            Self::Advanced => "advanced",
            Self::NoneSelected => "none",
        }
    }

    pub fn from_tag_value(value: &str) -> Option<Self> {
        match value.trim() {
            "monitored" => Some(Self::Monitored),
            "unmonitored" => Some(Self::Unmonitored),
            "futureepisodes" | "futureEpisodes" => Some(Self::FutureEpisodes),
            "missingandfutureepisodes" | "missingAndFutureEpisodes" => {
                Some(Self::MissingAndFutureEpisodes)
            }
            "allepisodes" | "allEpisodes" => Some(Self::AllEpisodes),
            "advanced" => Some(Self::Advanced),
            "none" => Some(Self::NoneSelected),
            _ => None,
        }
    }
}

/// Source form used to acquire a release.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadSourceKindValue {
    /// NZB supplied as a file.
    NzbFile,
    /// NZB supplied by URL.
    NzbUrl,
    /// Torrent supplied as a file.
    TorrentFile,
    /// Torrent supplied as a magnet URI.
    MagnetUri,
}

/// Reason a queued download was requested.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum QueueDownloadPurposeValue {
    /// Normal download request.
    Standard,
    /// Download requested for an additional file.
    AdditionalFile,
}

/// Path syntax used by the runtime.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum RuntimePathStyleValue {
    /// Unix path syntax.
    Unix,
    /// Windows path syntax.
    Windows,
}

/// Preferred download protocol for a delay profile.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DelayProfilePreferredProtocolValue {
    /// Prefer Usenet.
    Usenet,
    /// Prefer torrents.
    Torrent,
}

/// Processing state of a queued download.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadQueueStateValue {
    /// Queued but not started.
    Queued,
    /// Download is in progress.
    Downloading,
    /// Download is being verified.
    Verifying,
    /// Download is being repaired.
    Repairing,
    /// Download is being extracted.
    Extracting,
    /// Download is paused.
    Paused,
    /// Download completed.
    Completed,
    /// Download awaits import.
    ImportPending,
    /// The client reports a recoverable problem; the download is untouched by
    /// failed-download handling.
    Warning,
    /// Download failed.
    Failed,
}

impl DownloadQueueStateValue {
    pub fn from_domain(value: DownloadQueueState) -> Self {
        match value {
            DownloadQueueState::Queued => Self::Queued,
            DownloadQueueState::Downloading => Self::Downloading,
            DownloadQueueState::Verifying => Self::Verifying,
            DownloadQueueState::Repairing => Self::Repairing,
            DownloadQueueState::Extracting => Self::Extracting,
            DownloadQueueState::Paused => Self::Paused,
            DownloadQueueState::Completed => Self::Completed,
            DownloadQueueState::ImportPending => Self::ImportPending,
            DownloadQueueState::Warning => Self::Warning,
            DownloadQueueState::Failed => Self::Failed,
        }
    }
}

/// Display state combining download and post-processing lifecycle information.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadDisplayStateValue {
    /// Waiting in the download queue.
    Queued,
    /// Downloading data.
    Downloading,
    /// Paused by an operator or policy.
    Paused,
    /// Running post-processing.
    PostProcessing,
    /// Download and processing completed.
    Completed,
    /// Import completed, but the torrent remains managed while seeding.
    ImportedSeeding,
    /// Download or processing failed.
    Failed,
    /// The client reports a recoverable problem on a download that is still
    /// live.
    Warning,
    /// Import is running.
    Importing,
    /// Import is waiting to run.
    ImportPending,
    /// Import is blocked by policy or prerequisites.
    ImportBlocked,
    /// Import failed.
    ImportFailed,
    /// Item was ignored.
    Ignored,
    /// Removal is running.
    Removing,
    /// Removal failed.
    RemoveFailed,
}

/// Seeding obligation state for a torrent download.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadSeedingStateValue {
    /// Nothing to report yet: still downloading, or the client exposes no seeding state.
    None,
    /// Seeding towards an obligation that is not yet discharged.
    Seeding,
    /// The seeding obligation is discharged.
    GoalMet,
    /// Observed as a private torrent with no resolved goals, so it is never removed automatically.
    HeldPrivate,
    /// The seeding profile keeps this entry forever.
    NeverRemove,
}

/// Filter for active download activity.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadActivityFilterValue {
    /// Include every activity state.
    All,
    /// Include downloading items.
    Downloading,
    /// Include queued items.
    Queued,
    /// Include paused items.
    Paused,
    /// Include post-processing items.
    PostProcessing,
    /// Include imported torrents still retained for seeding.
    Seeding,
    /// Include items the client reported a recoverable problem for.
    Warning,
}

/// Filter for import activity.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadImportFilterValue {
    /// Include every import state.
    All,
    /// Include imports awaiting attention rather than actively executing.
    Attention,
    /// Include imports currently running.
    Importing,
    /// Include imports awaiting action.
    Pending,
    /// Include imports blocked by policy.
    Blocked,
    /// Include failed imports.
    Failed,
}

/// Filter for download history outcomes.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadHistoryFilterValue {
    /// Include all history entries.
    All,
    /// Include successful entries.
    Success,
    /// Include failed entries.
    Failed,
}

/// Sort key for download history.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadHistorySortKeyValue {
    /// Sort by title.
    Title,
    /// Sort by download client.
    Client,
    /// Sort by status.
    Status,
    /// Sort by progress.
    Progress,
    /// Sort by size.
    Size,
}

/// Sort key for the active download queue.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadQueueSortKeyValue {
    /// Sort by title.
    Title,
    /// Sort by download client.
    Client,
    /// Sort by status.
    Status,
    /// Sort by progress.
    Progress,
    /// Sort by size.
    Size,
}

/// Direction for sortable list results.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum SortDirectionValue {
    /// Lowest or earliest values first.
    Asc,
    /// Highest or latest values first.
    Desc,
}

/// Domain event name emitted by the application.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DomainEventTypeValue {
    /// A media request was submitted.
    MediaRequestSubmitted,
    /// A media request changed.
    MediaRequestUpdated,
    /// A media request was approved.
    MediaRequestApproved,
    /// A media request was rejected.
    MediaRequestRejected,
    /// A media request was canceled.
    MediaRequestCanceled,
    /// A title was added.
    TitleAdded,
    /// A title was updated.
    TitleUpdated,
    /// A title was rematched.
    TitleRematched,
    /// A title was deleted.
    TitleDeleted,
    /// Configuration changed.
    ConfigurationChanged,
    /// A discovery search completed.
    DiscoverySearchCompleted,
    /// Metadata hydration changed.
    MetadataHydrationUpdated,
    /// A release was grabbed.
    ReleaseGrabbed,
    /// A download failed.
    DownloadFailed,
    /// A download was ignored.
    DownloadIgnored,
    /// A release was blocklisted.
    ReleaseBlocklisted,
    /// An import completed.
    ImportCompleted,
    /// An import was rejected.
    ImportRejected,
    /// A media file was imported.
    MediaFileImported,
    /// A media file was analyzed.
    MediaFileAnalyzed,
    /// A media file was renamed.
    MediaFileRenamed,
    /// A media file was deleted.
    MediaFileDeleted,
    /// A media file was upgraded.
    MediaFileUpgraded,
    /// An acquisition search completed.
    AcquisitionSearchCompleted,
    /// An acquisition candidate was rejected.
    AcquisitionCandidateRejected,
    /// An import was requested.
    ImportRequested,
    /// Import recovery completed.
    ImportRecoveryCompleted,
    /// A download queue command was issued.
    DownloadQueueItemCommandIssued,
    /// Post-processing completed.
    PostProcessingCompleted,
    /// A subtitle was downloaded.
    SubtitleDownloaded,
    /// A subtitle search failed.
    SubtitleSearchFailed,
    /// A library scan started.
    LibraryScanStarted,
    /// A title was discovered during a library scan.
    LibraryScanTitleDiscovered,
    /// A library scan delta was recorded.
    LibraryScanDeltaRecorded,
    /// Library scan progress changed.
    LibraryScanProgressed,
    /// A library scan completed.
    LibraryScanCompleted,
    /// A library scan was canceled.
    LibraryScanCanceled,
    /// A library scan failed.
    LibraryScanFailed,
    /// A job run started.
    JobRunStarted,
    /// A job run completed.
    JobRunCompleted,
    /// A job run failed.
    JobRunFailed,
    /// A job's next-run time changed.
    JobNextRunUpdated,
    /// A download queue item was added or updated.
    DownloadQueueItemUpserted,
    /// A download queue item was removed.
    DownloadQueueItemRemoved,
    /// A torrent was imported and its client entry retained while it seeds.
    SeedingStarted,
    /// A torrent's seeding obligation was discharged.
    SeedingCompleted,
}

impl DomainEventTypeValue {
    pub fn from_domain(value: DomainEventType) -> Self {
        match value {
            DomainEventType::MediaRequestSubmitted => Self::MediaRequestSubmitted,
            DomainEventType::MediaRequestUpdated => Self::MediaRequestUpdated,
            DomainEventType::MediaRequestApproved => Self::MediaRequestApproved,
            DomainEventType::MediaRequestRejected => Self::MediaRequestRejected,
            DomainEventType::MediaRequestCanceled => Self::MediaRequestCanceled,
            DomainEventType::TitleAdded => Self::TitleAdded,
            DomainEventType::TitleUpdated => Self::TitleUpdated,
            DomainEventType::TitleRematched => Self::TitleRematched,
            DomainEventType::TitleDeleted => Self::TitleDeleted,
            DomainEventType::ConfigurationChanged => Self::ConfigurationChanged,
            DomainEventType::DiscoverySearchCompleted => Self::DiscoverySearchCompleted,
            DomainEventType::MetadataHydrationUpdated => Self::MetadataHydrationUpdated,
            DomainEventType::ReleaseGrabbed => Self::ReleaseGrabbed,
            DomainEventType::DownloadFailed => Self::DownloadFailed,
            DomainEventType::DownloadIgnored => Self::DownloadIgnored,
            DomainEventType::ReleaseBlocklisted => Self::ReleaseBlocklisted,
            DomainEventType::ImportCompleted => Self::ImportCompleted,
            DomainEventType::ImportRejected => Self::ImportRejected,
            DomainEventType::MediaFileImported => Self::MediaFileImported,
            DomainEventType::MediaFileAnalyzed => Self::MediaFileAnalyzed,
            DomainEventType::MediaFileRenamed => Self::MediaFileRenamed,
            DomainEventType::MediaFileDeleted => Self::MediaFileDeleted,
            DomainEventType::MediaFileUpgraded => Self::MediaFileUpgraded,
            DomainEventType::AcquisitionSearchCompleted => Self::AcquisitionSearchCompleted,
            DomainEventType::AcquisitionCandidateRejected => Self::AcquisitionCandidateRejected,
            DomainEventType::ImportRequested => Self::ImportRequested,
            DomainEventType::ImportRecoveryCompleted => Self::ImportRecoveryCompleted,
            DomainEventType::DownloadQueueItemCommandIssued => Self::DownloadQueueItemCommandIssued,
            DomainEventType::PostProcessingCompleted => Self::PostProcessingCompleted,
            DomainEventType::SubtitleDownloaded => Self::SubtitleDownloaded,
            DomainEventType::SubtitleSearchFailed => Self::SubtitleSearchFailed,
            DomainEventType::LibraryScanStarted => Self::LibraryScanStarted,
            DomainEventType::LibraryScanTitleDiscovered => Self::LibraryScanTitleDiscovered,
            DomainEventType::LibraryScanDeltaRecorded => Self::LibraryScanDeltaRecorded,
            DomainEventType::LibraryScanProgressed => Self::LibraryScanProgressed,
            DomainEventType::LibraryScanCompleted => Self::LibraryScanCompleted,
            DomainEventType::LibraryScanCanceled => Self::LibraryScanCanceled,
            DomainEventType::LibraryScanFailed => Self::LibraryScanFailed,
            DomainEventType::JobRunStarted => Self::JobRunStarted,
            DomainEventType::JobRunCompleted => Self::JobRunCompleted,
            DomainEventType::JobRunFailed => Self::JobRunFailed,
            DomainEventType::JobNextRunUpdated => Self::JobNextRunUpdated,
            DomainEventType::DownloadQueueItemUpserted => Self::DownloadQueueItemUpserted,
            DomainEventType::DownloadQueueItemRemoved => Self::DownloadQueueItemRemoved,
            DomainEventType::SeedingStarted => Self::SeedingStarted,
            DomainEventType::SeedingCompleted => Self::SeedingCompleted,
        }
    }

    pub fn into_domain(self) -> DomainEventType {
        match self {
            Self::MediaRequestSubmitted => DomainEventType::MediaRequestSubmitted,
            Self::MediaRequestUpdated => DomainEventType::MediaRequestUpdated,
            Self::MediaRequestApproved => DomainEventType::MediaRequestApproved,
            Self::MediaRequestRejected => DomainEventType::MediaRequestRejected,
            Self::MediaRequestCanceled => DomainEventType::MediaRequestCanceled,
            Self::TitleAdded => DomainEventType::TitleAdded,
            Self::TitleUpdated => DomainEventType::TitleUpdated,
            Self::TitleRematched => DomainEventType::TitleRematched,
            Self::TitleDeleted => DomainEventType::TitleDeleted,
            Self::ConfigurationChanged => DomainEventType::ConfigurationChanged,
            Self::DiscoverySearchCompleted => DomainEventType::DiscoverySearchCompleted,
            Self::MetadataHydrationUpdated => DomainEventType::MetadataHydrationUpdated,
            Self::ReleaseGrabbed => DomainEventType::ReleaseGrabbed,
            Self::DownloadFailed => DomainEventType::DownloadFailed,
            Self::DownloadIgnored => DomainEventType::DownloadIgnored,
            Self::ReleaseBlocklisted => DomainEventType::ReleaseBlocklisted,
            Self::ImportCompleted => DomainEventType::ImportCompleted,
            Self::ImportRejected => DomainEventType::ImportRejected,
            Self::MediaFileImported => DomainEventType::MediaFileImported,
            Self::MediaFileAnalyzed => DomainEventType::MediaFileAnalyzed,
            Self::MediaFileRenamed => DomainEventType::MediaFileRenamed,
            Self::MediaFileDeleted => DomainEventType::MediaFileDeleted,
            Self::MediaFileUpgraded => DomainEventType::MediaFileUpgraded,
            Self::AcquisitionSearchCompleted => DomainEventType::AcquisitionSearchCompleted,
            Self::AcquisitionCandidateRejected => DomainEventType::AcquisitionCandidateRejected,
            Self::ImportRequested => DomainEventType::ImportRequested,
            Self::ImportRecoveryCompleted => DomainEventType::ImportRecoveryCompleted,
            Self::DownloadQueueItemCommandIssued => DomainEventType::DownloadQueueItemCommandIssued,
            Self::PostProcessingCompleted => DomainEventType::PostProcessingCompleted,
            Self::SubtitleDownloaded => DomainEventType::SubtitleDownloaded,
            Self::SubtitleSearchFailed => DomainEventType::SubtitleSearchFailed,
            Self::LibraryScanStarted => DomainEventType::LibraryScanStarted,
            Self::LibraryScanTitleDiscovered => DomainEventType::LibraryScanTitleDiscovered,
            Self::LibraryScanDeltaRecorded => DomainEventType::LibraryScanDeltaRecorded,
            Self::LibraryScanProgressed => DomainEventType::LibraryScanProgressed,
            Self::LibraryScanCompleted => DomainEventType::LibraryScanCompleted,
            Self::LibraryScanCanceled => DomainEventType::LibraryScanCanceled,
            Self::LibraryScanFailed => DomainEventType::LibraryScanFailed,
            Self::JobRunStarted => DomainEventType::JobRunStarted,
            Self::JobRunCompleted => DomainEventType::JobRunCompleted,
            Self::JobRunFailed => DomainEventType::JobRunFailed,
            Self::JobNextRunUpdated => DomainEventType::JobNextRunUpdated,
            Self::DownloadQueueItemUpserted => DomainEventType::DownloadQueueItemUpserted,
            Self::DownloadQueueItemRemoved => DomainEventType::DownloadQueueItemRemoved,
            Self::SeedingStarted => DomainEventType::SeedingStarted,
            Self::SeedingCompleted => DomainEventType::SeedingCompleted,
        }
    }
}

/// Lifecycle state of a tracked download.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum TrackedDownloadStateValue {
    /// Download is in progress.
    Downloading,
    /// Download awaits import.
    ImportPending,
    /// Import is in progress.
    Importing,
    /// Import completed successfully.
    Imported,
    /// Import completed, but the torrent is still seeding toward its goal and
    /// cannot be removed from the client yet.
    ImportedSeeding,
    /// Import is blocked.
    ImportBlocked,
    /// Failure awaits retry or handling.
    FailedPending,
    /// Download or import failed.
    Failed,
    /// Download was ignored.
    Ignored,
}

impl TrackedDownloadStateValue {
    pub fn from_domain(value: TrackedDownloadState) -> Self {
        match value {
            TrackedDownloadState::Downloading => Self::Downloading,
            TrackedDownloadState::ImportPending => Self::ImportPending,
            TrackedDownloadState::Importing => Self::Importing,
            TrackedDownloadState::Imported => Self::Imported,
            TrackedDownloadState::ImportedSeeding => Self::ImportedSeeding,
            TrackedDownloadState::ImportBlocked => Self::ImportBlocked,
            TrackedDownloadState::FailedPending => Self::FailedPending,
            TrackedDownloadState::Failed => Self::Failed,
            TrackedDownloadState::Ignored => Self::Ignored,
        }
    }
}

/// Severity of a tracked-download health result.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum TrackedDownloadStatusValue {
    /// No issue was detected.
    Ok,
    /// The item needs attention but is not failed.
    Warning,
    /// The item has an error.
    Error,
}

impl TrackedDownloadStatusValue {
    pub fn from_domain(value: TrackedDownloadStatus) -> Self {
        match value {
            TrackedDownloadStatus::Ok => Self::Ok,
            TrackedDownloadStatus::Warning => Self::Warning,
            TrackedDownloadStatus::Error => Self::Error,
        }
    }
}

/// Source used to match a release to a title.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum TitleMatchTypeValue {
    /// Matched from the original submission.
    Submission,
    /// Matched from a client-supplied parameter.
    ClientParameter,
    /// Matched by parsing the title.
    TitleParse,
    /// Matched by an explicit ID.
    IdOnly,
    /// No title match was found.
    Unmatched,
}

impl TitleMatchTypeValue {
    pub fn from_domain(value: TitleMatchType) -> Self {
        match value {
            TitleMatchType::Submission => Self::Submission,
            TitleMatchType::ClientParameter => Self::ClientParameter,
            TitleMatchType::TitleParse => Self::TitleParse,
            TitleMatchType::IdOnly => Self::IdOnly,
            TitleMatchType::Unmatched => Self::Unmatched,
        }
    }
}

/// Lifecycle state of an import operation.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportStatusValue {
    /// Import is queued.
    Pending,
    /// Import work is running.
    Running,
    /// Import is processing results.
    Processing,
    /// Import completed successfully.
    Completed,
    /// Import failed.
    Failed,
    /// Import was skipped.
    Skipped,
}

impl ImportStatusValue {
    pub fn from_domain(value: ImportStatus) -> Self {
        match value {
            ImportStatus::Pending => Self::Pending,
            ImportStatus::Running => Self::Running,
            ImportStatus::Processing => Self::Processing,
            ImportStatus::Completed => Self::Completed,
            ImportStatus::Failed => Self::Failed,
            ImportStatus::Skipped => Self::Skipped,
        }
    }
}

/// Kind of import or rename operation.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportTypeValue {
    /// Movie download import.
    MovieDownload,
    /// Series download import.
    SeriesDownload,
    /// User-requested manual import.
    ManualImport,
    /// Rename plan preview.
    RenamePreview,
    /// Rename operation for one title.
    RenameApplyTitle,
    /// Rename operation for one facet.
    RenameApplyFacet,
    /// Rename operation result handling.
    RenameApplyResult,
    /// Rename operation failed during I/O.
    RenameIoFailed,
    /// Rename operation moved a file.
    RenameMove,
    /// Rename plan was stale.
    RenameStalePlan,
}

impl ImportTypeValue {
    pub fn from_domain(value: ImportType) -> Self {
        match value {
            ImportType::MovieDownload => Self::MovieDownload,
            ImportType::SeriesDownload => Self::SeriesDownload,
            ImportType::ManualImport => Self::ManualImport,
            ImportType::RenamePreview => Self::RenamePreview,
            ImportType::RenameApplyTitle => Self::RenameApplyTitle,
            ImportType::RenameApplyFacet => Self::RenameApplyFacet,
            ImportType::RenameApplyResult => Self::RenameApplyResult,
            ImportType::RenameIoFailed => Self::RenameIoFailed,
            ImportType::RenameMove => Self::RenameMove,
            ImportType::RenameStalePlan => Self::RenameStalePlan,
        }
    }
}

/// Machine-readable reason an import failed.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportErrorCodeValue {
    /// Source file was not found.
    FileNotFound,
    /// Target episode was not found.
    EpisodeNotFound,
    /// Episode lookup failed.
    EpisodeLookupFailed,
    /// The source job failed.
    SourceJobFailed,
    /// Import policy did not match.
    PolicyMismatch,
    /// File I/O failed.
    IoFailed,
    /// Permission denied.
    PermissionDenied,
    /// Storage is full.
    DiskFull,
    /// Failure has no more specific code.
    Unknown,
}

impl ImportErrorCodeValue {
    pub fn from_domain(value: ImportErrorCode) -> Self {
        match value {
            ImportErrorCode::FileNotFound => Self::FileNotFound,
            ImportErrorCode::EpisodeNotFound => Self::EpisodeNotFound,
            ImportErrorCode::EpisodeLookupFailed => Self::EpisodeLookupFailed,
            ImportErrorCode::SourceJobFailed => Self::SourceJobFailed,
            ImportErrorCode::PolicyMismatch => Self::PolicyMismatch,
            ImportErrorCode::IoFailed => Self::IoFailed,
            ImportErrorCode::PermissionDenied => Self::PermissionDenied,
            ImportErrorCode::DiskFull => Self::DiskFull,
            ImportErrorCode::Unknown => Self::Unknown,
        }
    }
}

/// State of a queued download deletion request.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadQueueDeleteStatusValue {
    /// Deletion is queued.
    Queued,
    /// Deletion is running.
    Running,
    /// Deletion completed.
    Completed,
    /// Deletion failed.
    Failed,
}

impl DownloadQueueDeleteStatusValue {
    pub fn from_domain(value: scryer_domain::DownloadQueueDeleteStatus) -> Self {
        match value {
            scryer_domain::DownloadQueueDeleteStatus::Queued => Self::Queued,
            scryer_domain::DownloadQueueDeleteStatus::Running => Self::Running,
            scryer_domain::DownloadQueueDeleteStatus::Completed => Self::Completed,
            scryer_domain::DownloadQueueDeleteStatus::Failed => Self::Failed,
        }
    }
}

/// Outcome of an import decision.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportDecisionValue {
    /// File was imported.
    Imported,
    /// File was rejected.
    Rejected,
    /// File was skipped.
    Skipped,
    /// File conflicted with another candidate.
    Conflict,
    /// File could not be matched.
    Unmatched,
    /// Import failed.
    Failed,
}

impl ImportDecisionValue {
    pub fn from_domain(value: ImportDecision) -> Self {
        match value {
            ImportDecision::Imported => Self::Imported,
            ImportDecision::Rejected => Self::Rejected,
            ImportDecision::Skipped => Self::Skipped,
            ImportDecision::Conflict => Self::Conflict,
            ImportDecision::Unmatched => Self::Unmatched,
            ImportDecision::Failed => Self::Failed,
        }
    }
}

/// Reason an import candidate was skipped.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportSkipReasonValue {
    /// File was already imported.
    AlreadyImported,
    /// File duplicated another candidate.
    DuplicateFile,
    /// Post-download rules blocked the file.
    PostDownloadRuleBlocked,
    /// Import policy did not match.
    PolicyMismatch,
    /// Title identity could not be resolved.
    UnresolvedIdentity,
    /// Episode metadata could not be parsed.
    UnparseableEpisode,
    /// No video files were found.
    NoVideoFiles,
    /// The download client is still writing or unpacking the source file.
    DownloadInProgress,
    /// Storage is full.
    DiskFull,
    /// Permission was denied.
    PermissionDenied,
    /// A password is required before retrying.
    PasswordRequired,
    /// An archive extractor must be installed or enabled before retrying.
    ArchiveExtractionPluginRequired,
    /// Archive extraction exceeded its configured time limit.
    ArchiveExtractionTimedOut,
}

impl ImportSkipReasonValue {
    pub fn from_domain(value: ImportSkipReason) -> Self {
        match value {
            ImportSkipReason::AlreadyImported => Self::AlreadyImported,
            ImportSkipReason::DuplicateFile => Self::DuplicateFile,
            ImportSkipReason::PostDownloadRuleBlocked => Self::PostDownloadRuleBlocked,
            ImportSkipReason::PolicyMismatch => Self::PolicyMismatch,
            ImportSkipReason::UnresolvedIdentity => Self::UnresolvedIdentity,
            ImportSkipReason::UnparseableEpisode => Self::UnparseableEpisode,
            ImportSkipReason::NoVideoFiles => Self::NoVideoFiles,
            ImportSkipReason::DownloadInProgress => Self::DownloadInProgress,
            ImportSkipReason::DiskFull => Self::DiskFull,
            ImportSkipReason::PermissionDenied => Self::PermissionDenied,
            ImportSkipReason::PasswordRequired => Self::PasswordRequired,
            ImportSkipReason::ArchiveExtractionPluginRequired => {
                Self::ArchiveExtractionPluginRequired
            }
            ImportSkipReason::ArchiveExtractionTimedOut => Self::ArchiveExtractionTimedOut,
        }
    }
}

/// Policy for filler episodes or scenes.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum FillerPolicyValue {
    /// Download filler content.
    DownloadAll,
    /// Skip filler content.
    SkipFiller,
}

impl FillerPolicyValue {
    // Stored as a settings string / structured title tag by the application.
    pub fn as_app_str(self) -> &'static str {
        match self {
            Self::DownloadAll => "download_all",
            Self::SkipFiller => "skip_filler",
        }
    }

    pub fn from_app_str(value: &str) -> Option<Self> {
        match value {
            "download_all" => Some(Self::DownloadAll),
            "skip_filler" => Some(Self::SkipFiller),
            _ => None,
        }
    }
}

/// Policy for recap episodes or scenes.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum RecapPolicyValue {
    /// Download recap content.
    DownloadAll,
    /// Skip recap content.
    SkipRecap,
}

impl RecapPolicyValue {
    pub fn as_app_str(self) -> &'static str {
        match self {
            Self::DownloadAll => "download_all",
            Self::SkipRecap => "skip_recap",
        }
    }

    pub fn from_app_str(value: &str) -> Option<Self> {
        match value {
            "download_all" => Some(Self::DownloadAll),
            "skip_recap" => Some(Self::SkipRecap),
            _ => None,
        }
    }
}

/// Phase of transferring an imported file.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportTransferPhaseValue {
    /// Archive contents are being extracted before file transfer.
    Extracting,
    /// File bytes are being copied.
    Copying,
    /// Transfer is finalizing metadata and filesystem state.
    Finalizing,
}

impl From<ImportTransferPhase> for ImportTransferPhaseValue {
    fn from(value: ImportTransferPhase) -> Self {
        match value {
            ImportTransferPhase::Extracting => Self::Extracting,
            ImportTransferPhase::Copying => Self::Copying,
            ImportTransferPhase::Finalizing => Self::Finalizing,
        }
    }
}

/// Health state reported by the plugin catalog runtime.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum CatalogRefreshStateValue {
    /// Catalog is ready for use.
    Ready,
    /// Catalog is usable with degraded availability.
    Degraded,
}

impl CatalogRefreshStateValue {
    // The plugin-catalog runtime reports this as a string; parse at the API
    // boundary and fail safe to Ready.
    pub fn from_app_str(value: &str) -> Self {
        match value {
            "degraded" => Self::Degraded,
            _ => Self::Ready,
        }
    }
}

/// Stream identifier scope for event subscriptions.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum StreamKindValue {
    /// Events from the global stream.
    Global,
    /// Events for one title.
    Title,
    /// Events for one library scan.
    LibraryScan,
    /// Events for one job run.
    JobRun,
    /// Events for one download queue item.
    DownloadQueueItem,
}

/// File operation used during import.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportModeValue {
    /// Hardlink when possible, otherwise copy.
    HardlinkOrCopy,
    /// Move the source file.
    Move,
}

impl From<ImportMode> for ImportModeValue {
    fn from(value: ImportMode) -> Self {
        match value {
            ImportMode::HardlinkOrCopy => Self::HardlinkOrCopy,
            ImportMode::Move => Self::Move,
        }
    }
}

impl From<ImportModeValue> for ImportMode {
    fn from(value: ImportModeValue) -> Self {
        match value {
            ImportModeValue::HardlinkOrCopy => Self::HardlinkOrCopy,
            ImportModeValue::Move => Self::Move,
        }
    }
}

/// Action when a rename destination already exists.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum RenameCollisionPolicyValue {
    /// Leave the existing destination and skip the rename.
    Skip,
    /// Treat an existing destination as an error.
    Error,
    /// Replace only when the incoming file is better.
    ReplaceIfBetter,
}

impl RenameCollisionPolicyValue {
    // The application/settings layer stores these as canonical strings; the
    // enum exists at the API boundary only.
    pub fn as_app_str(self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Error => "error",
            Self::ReplaceIfBetter => "replace_if_better",
        }
    }

    pub fn from_app_str(value: &str) -> Option<Self> {
        // Tolerant like the application-layer parser (trim + case-insensitive):
        // the stored value is a raw settings string.
        match value.trim().to_ascii_lowercase().as_str() {
            "skip" => Some(Self::Skip),
            "error" => Some(Self::Error),
            "replace_if_better" => Some(Self::ReplaceIfBetter),
            _ => None,
        }
    }
}

/// Fallback when required metadata is missing during rename.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum RenameMissingMetadataPolicyValue {
    /// Skip the rename.
    Skip,
    /// Use the title as fallback metadata.
    FallbackTitle,
}

impl RenameMissingMetadataPolicyValue {
    pub fn as_app_str(self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::FallbackTitle => "fallback_title",
        }
    }

    pub fn from_app_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "skip" => Some(Self::Skip),
            "fallback_title" => Some(Self::FallbackTitle),
            _ => None,
        }
    }
}

/// Collection grouping type.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum CollectionTypeValue {
    /// A season grouping.
    Season,
    /// A movie grouping.
    Movie,
    /// An arc grouping.
    Arc,
    /// A specials grouping.
    Specials,
}

impl From<CollectionType> for CollectionTypeValue {
    fn from(value: CollectionType) -> Self {
        match value {
            CollectionType::Season => Self::Season,
            CollectionType::Movie => Self::Movie,
            CollectionType::Arc => Self::Arc,
            CollectionType::Specials => Self::Specials,
        }
    }
}

/// Episode classification.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum EpisodeTypeValue {
    /// Standard episode.
    Standard,
    /// Special episode.
    Special,
    /// Official episode classification.
    Official,
    /// Original video animation episode.
    Ova,
    /// Original net animation episode.
    Ona,
    /// Alternate episode version.
    Alternate,
}

impl From<EpisodeType> for EpisodeTypeValue {
    fn from(value: EpisodeType) -> Self {
        match value {
            EpisodeType::Standard => Self::Standard,
            EpisodeType::Special => Self::Special,
            EpisodeType::Official => Self::Official,
            EpisodeType::Ova => Self::Ova,
            EpisodeType::Ona => Self::Ona,
            EpisodeType::Alternate => Self::Alternate,
        }
    }
}

/// Actor origin attached to an event.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ActorKindValue {
    /// A registered user caused the event.
    User,
    /// An unauthenticated actor caused the event.
    Anonymous,
    /// The system caused the event.
    System,
}

impl From<DomainEventActorKind> for ActorKindValue {
    fn from(value: DomainEventActorKind) -> Self {
        match value {
            DomainEventActorKind::User => Self::User,
            DomainEventActorKind::Anonymous => Self::Anonymous,
            DomainEventActorKind::System => Self::System,
        }
    }
}

/// Execution strategy for a job or script.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionModeValue {
    /// Complete work before returning.
    Blocking,
    /// Start work and return without waiting for completion.
    FireAndForget,
}

impl From<ExecutionMode> for ExecutionModeValue {
    fn from(value: ExecutionMode) -> Self {
        match value {
            ExecutionMode::Blocking => Self::Blocking,
            ExecutionMode::FireAndForget => Self::FireAndForget,
        }
    }
}

impl From<ExecutionModeValue> for ExecutionMode {
    fn from(value: ExecutionModeValue) -> Self {
        match value {
            ExecutionModeValue::Blocking => Self::Blocking,
            ExecutionModeValue::FireAndForget => Self::FireAndForget,
        }
    }
}

/// Kind of activity event shown to clients.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ActivityKindValue {
    /// A setting was saved.
    SettingSaved,
    /// A movie was fetched.
    MovieFetched,
    /// A title was added.
    TitleAdded,
    /// A title was updated.
    TitleUpdated,
    /// Metadata hydration started.
    MetadataHydrationStarted,
    /// Metadata hydration completed.
    MetadataHydrationCompleted,
    /// Metadata hydration failed.
    MetadataHydrationFailed,
    /// A movie was downloaded.
    MovieDownloaded,
    /// A series episode was imported.
    SeriesEpisodeImported,
    /// An acquisition search completed.
    AcquisitionSearchCompleted,
    /// An acquisition candidate was accepted.
    AcquisitionCandidateAccepted,
    /// An acquisition candidate was rejected.
    AcquisitionCandidateRejected,
    /// An acquisition download failed.
    AcquisitionDownloadFailed,
    /// Post-processing completed.
    PostProcessingCompleted,
    /// A file was analyzed.
    FileAnalyzed,
    /// A file was upgraded.
    FileUpgraded,
    /// An import was rejected.
    ImportRejected,
    /// A subtitle was downloaded.
    SubtitleDownloaded,
    /// A subtitle search failed.
    SubtitleSearchFailed,
    /// A system notice was emitted.
    SystemNotice,
}

/// Severity of an activity event.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ActivitySeverityValue {
    /// Informational event.
    Info,
    /// Successful operation.
    Success,
    /// Operation needs attention.
    Warning,
    /// Operation failed.
    Error,
}

/// Delivery channel associated with an activity event.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ActivityChannelValue {
    /// Web application activity channel.
    WebUi,
    /// Toast notification channel.
    Toast,
}

/// Lifecycle state of a wanted acquisition target.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum WantedStatusValue {
    /// Target is wanted and not yet grabbed.
    Wanted,
    /// A release was grabbed for the target.
    Grabbed,
    /// Target processing is paused.
    Paused,
    /// Target has completed acquisition.
    Completed,
}

impl WantedStatusValue {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wanted => "wanted",
            Self::Grabbed => "grabbed",
            Self::Paused => "paused",
            Self::Completed => "completed",
        }
    }
}

/// Media shape represented by a wanted target.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum WantedMediaTypeValue {
    /// A movie target.
    Movie,
    /// An episode target.
    Episode,
    /// A movie belonging to a series.
    SeriesMovie,
}

impl WantedMediaTypeValue {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Episode => "episode",
            Self::SeriesMovie => "series_movie",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "movie" => Some(Self::Movie),
            "episode" => Some(Self::Episode),
            "series_movie" => Some(Self::SeriesMovie),
            _ => None,
        }
    }
}

/// Search and RSS convergence state of an acquisition scope.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ConvergenceStateValue {
    /// No indexer has been searched under the current fingerprint and the cursor has not begun.
    Queued,
    /// Some, but not all, routed indexers are covered and the sweep is in progress.
    Searching,
    /// Every routed indexer is covered under the current fingerprint and RSS watches the scope.
    Converged,
    /// The scope is not converged and every uncovered indexer is currently unavailable.
    Deferred,
}

/// Recency lane used to prioritize acquisition convergence.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum RecencyLaneValue {
    /// Prioritized for prompt convergence.
    Hot,
    /// Drained under backpressure.
    Cold,
}

/// Derived acquisition-target set represented by a wanted view.
#[derive(Enum, Copy, Clone, Eq, PartialEq, Default)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum WantedKindValue {
    #[default]
    /// Scope has no primary file; this is the default target set.
    Missing,
    /// Scope has a file below the effective profile cutoff.
    CutoffUpgrade,
}

impl WantedKindValue {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::CutoffUpgrade => "cutoff_upgrade",
        }
    }
}

/// Lifecycle state of a pending release candidate.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum PendingReleaseStatusValue {
    /// Candidate is waiting for processing.
    Waiting,
    /// Candidate is held in standby.
    Standby,
    /// Candidate is being processed.
    Processing,
    /// Candidate was grabbed.
    Grabbed,
    /// Candidate was superseded.
    Superseded,
    /// Candidate expired before processing.
    Expired,
    /// Candidate was dismissed.
    Dismissed,
    /// Candidate needs manual review.
    NeedsReview,
}

/// Arbitration role of an active pending release candidate.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum PendingReleaseRoleValue {
    /// The highest-ranked active candidate for its overlap group.
    Primary,
    /// A lower-ranked active candidate retained as a fallback.
    Fallback,
}
