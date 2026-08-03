use async_graphql::{
    Enum, ID, InputObject, InputValueError, InputValueResult, Json, MaybeUndefined, OneofObject,
    Scalar, ScalarType, SimpleObject, Value,
};
use chrono::{DateTime, NaiveDate, Utc};
use scryer_domain::{
    AppPermission, CollectionType, ConfigFieldRole, ConfigFieldType, ConfigFieldValueSource,
    DomainEventActorKind, DomainEventType, DownloadQueueState, EpisodeType, ExecutionMode,
    ImportDecision, ImportErrorCode, ImportMode, ImportSkipReason, ImportStatus,
    ImportTransferPhase, ImportType, LibraryPermission, MediaFacet, MediaRequestStatus,
    TitleHistoryEventType, TitleMatchType, TrackedDownloadState, TrackedDownloadStatus,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MediaFacetValue {
    Movie,
    Series,
    Anime,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LibraryPermissionValue {
    View,
    ManageTitles,
    ResolveImports,
    ManageLibrary,
    Request,
    AutoApproveRequests,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum AppPermissionValue {
    ManageUsers,
    ManagePermissions,
    ManageSystemSettings,
    ManageCatalogSettings,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum UserAccountKindValue {
    Local,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum PendingImportStatusValue {
    Pending,
    Ignored,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MediaRequestStatusValue {
    Pending,
    Approved,
    Rejected,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ContentScopeValue {
    Movie,
    Series,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ScoringPersonaValue {
    Balanced,
    Audiophile,
    Efficient,
    Compatible,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MonitorTypeValue {
    Monitored,
    Unmonitored,
    FutureEpisodes,
    MissingAndFutureEpisodes,
    AllEpisodes,
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
            "none" => Some(Self::NoneSelected),
            _ => None,
        }
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadSourceKindValue {
    NzbFile,
    NzbUrl,
    TorrentFile,
    MagnetUri,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum QueueDownloadPurposeValue {
    Standard,
    AdditionalFile,
}

#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum RuntimePathStyleValue {
    Unix,
    Windows,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DelayProfilePreferredProtocolValue {
    Usenet,
    Torrent,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadQueueStateValue {
    Queued,
    Downloading,
    Verifying,
    Repairing,
    Extracting,
    Paused,
    Completed,
    ImportPending,
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
            DownloadQueueState::Failed => Self::Failed,
        }
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadDisplayStateValue {
    Queued,
    Downloading,
    Paused,
    PostProcessing,
    Completed,
    Failed,
    Importing,
    ImportPending,
    ImportBlocked,
    ImportFailed,
    Ignored,
    Removing,
    RemoveFailed,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadActivityFilterValue {
    All,
    Downloading,
    Queued,
    Paused,
    PostProcessing,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadImportFilterValue {
    All,
    Importing,
    Pending,
    Blocked,
    Failed,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadHistoryFilterValue {
    All,
    Success,
    Failed,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadHistorySortKeyValue {
    Title,
    Client,
    Status,
    Progress,
    Size,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum SortDirectionValue {
    Asc,
    Desc,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DomainEventTypeValue {
    MediaRequestSubmitted,
    MediaRequestUpdated,
    MediaRequestApproved,
    MediaRequestRejected,
    MediaRequestCanceled,
    TitleAdded,
    TitleUpdated,
    TitleRematched,
    TitleDeleted,
    ConfigurationChanged,
    DiscoverySearchCompleted,
    MetadataHydrationUpdated,
    ReleaseGrabbed,
    DownloadFailed,
    DownloadIgnored,
    ReleaseBlocklisted,
    ImportCompleted,
    ImportRejected,
    MediaFileImported,
    MediaFileAnalyzed,
    MediaFileRenamed,
    MediaFileDeleted,
    MediaFileUpgraded,
    AcquisitionSearchCompleted,
    AcquisitionCandidateRejected,
    ImportRequested,
    ImportRecoveryCompleted,
    DownloadQueueItemCommandIssued,
    PostProcessingCompleted,
    SubtitleDownloaded,
    SubtitleSearchFailed,
    LibraryScanStarted,
    LibraryScanTitleDiscovered,
    LibraryScanDeltaRecorded,
    LibraryScanProgressed,
    LibraryScanCompleted,
    LibraryScanCanceled,
    LibraryScanFailed,
    JobRunStarted,
    JobRunCompleted,
    JobRunFailed,
    JobNextRunUpdated,
    DownloadQueueItemUpserted,
    DownloadQueueItemRemoved,
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
        }
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum TrackedDownloadStateValue {
    Downloading,
    ImportPending,
    Importing,
    Imported,
    ImportBlocked,
    FailedPending,
    Failed,
    Ignored,
}

impl TrackedDownloadStateValue {
    pub fn from_domain(value: TrackedDownloadState) -> Self {
        match value {
            TrackedDownloadState::Downloading => Self::Downloading,
            TrackedDownloadState::ImportPending => Self::ImportPending,
            TrackedDownloadState::Importing => Self::Importing,
            TrackedDownloadState::Imported => Self::Imported,
            TrackedDownloadState::ImportBlocked => Self::ImportBlocked,
            TrackedDownloadState::FailedPending => Self::FailedPending,
            TrackedDownloadState::Failed => Self::Failed,
            TrackedDownloadState::Ignored => Self::Ignored,
        }
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum TrackedDownloadStatusValue {
    Ok,
    Warning,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum TitleMatchTypeValue {
    Submission,
    ClientParameter,
    TitleParse,
    IdOnly,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportStatusValue {
    Pending,
    Running,
    Processing,
    Completed,
    Failed,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportTypeValue {
    MovieDownload,
    SeriesDownload,
    ManualImport,
    RenamePreview,
    RenameApplyTitle,
    RenameApplyFacet,
    RenameApplyResult,
    RenameIoFailed,
    RenameMove,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportErrorCodeValue {
    FileNotFound,
    EpisodeNotFound,
    EpisodeLookupFailed,
    SourceJobFailed,
    PolicyMismatch,
    IoFailed,
    PermissionDenied,
    DiskFull,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadQueueDeleteStatusValue {
    Queued,
    Running,
    Completed,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportDecisionValue {
    Imported,
    Rejected,
    Skipped,
    Conflict,
    Unmatched,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportSkipReasonValue {
    AlreadyImported,
    DuplicateFile,
    PostDownloadRuleBlocked,
    PolicyMismatch,
    UnresolvedIdentity,
    UnparseableEpisode,
    NoVideoFiles,
    DiskFull,
    PermissionDenied,
    PasswordRequired,
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
            ImportSkipReason::DiskFull => Self::DiskFull,
            ImportSkipReason::PermissionDenied => Self::PermissionDenied,
            ImportSkipReason::PasswordRequired => Self::PasswordRequired,
        }
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum FillerPolicyValue {
    DownloadAll,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum RecapPolicyValue {
    DownloadAll,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportTransferPhaseValue {
    Copying,
    Finalizing,
}

impl From<ImportTransferPhase> for ImportTransferPhaseValue {
    fn from(value: ImportTransferPhase) -> Self {
        match value {
            ImportTransferPhase::Copying => Self::Copying,
            ImportTransferPhase::Finalizing => Self::Finalizing,
        }
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum CatalogRefreshStateValue {
    Ready,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum StreamKindValue {
    Global,
    Title,
    LibraryScan,
    JobRun,
    DownloadQueueItem,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ImportModeValue {
    HardlinkOrCopy,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum RenameCollisionPolicyValue {
    Skip,
    Error,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum RenameMissingMetadataPolicyValue {
    Skip,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum CollectionTypeValue {
    Season,
    Movie,
    Arc,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum EpisodeTypeValue {
    Standard,
    Special,
    Official,
    Ova,
    Ona,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ActorKindValue {
    User,
    Anonymous,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionModeValue {
    Blocking,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ActivityKindValue {
    SettingSaved,
    MovieFetched,
    TitleAdded,
    TitleUpdated,
    MetadataHydrationStarted,
    MetadataHydrationCompleted,
    MetadataHydrationFailed,
    MovieDownloaded,
    SeriesEpisodeImported,
    AcquisitionSearchCompleted,
    AcquisitionCandidateAccepted,
    AcquisitionCandidateRejected,
    AcquisitionDownloadFailed,
    PostProcessingCompleted,
    FileAnalyzed,
    FileUpgraded,
    ImportRejected,
    SubtitleDownloaded,
    SubtitleSearchFailed,
    SystemNotice,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ActivitySeverityValue {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ActivityChannelValue {
    WebUi,
    Toast,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum WantedStatusValue {
    Wanted,
    Grabbed,
    Paused,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum WantedMediaTypeValue {
    Movie,
    Episode,
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

/// Convergence state of an acquisition scope. Replaces the retired
/// cadence display (`searchPhase`/`nextSearchAt`): the operator sees whether a scope
/// is still sweeping indexers, has converged onto RSS, is queued for the cursor, or
/// is deferred because every uncovered indexer is currently unavailable.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ConvergenceStateValue {
    /// No indexer has been searched under the current fingerprint yet — waiting on
    /// the cursor to begin the sweep.
    Queued,
    /// Some but not all routed indexers are covered — the convergence sweep is in
    /// progress.
    Searching,
    /// Every routed indexer is covered under the current fingerprint — the scope is
    /// watched via RSS and only re-checked on demand.
    Converged,
    /// Not converged, and every still-uncovered indexer is currently unavailable
    /// (per the scheduler snapshot — cooling down or out of quota).
    Deferred,
}

/// Recency lane of an acquisition scope: `Hot` scopes converge
/// promptly (high candidate value to the scheduler); `Cold` scopes drain under
/// backpressure.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum RecencyLaneValue {
    Hot,
    Cold,
}

/// Which derived acquisition-target set a wanted view lists:
/// `Missing` scopes have no primary file; `CutoffUpgrade` scopes have a file below
/// the effective profile cutoff.
#[derive(Enum, Copy, Clone, Eq, PartialEq, Default)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum WantedKindValue {
    #[default]
    Missing,
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

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum PendingReleaseStatusValue {
    Waiting,
    Standby,
    Processing,
    Grabbed,
    Superseded,
    Expired,
    Dismissed,
    NeedsReview,
}

#[derive(InputObject)]
pub struct LoginInput {
    pub username: String,
    pub password: String,
    pub totp_code: Option<String>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ExternalAccountProviderValue {
    Plex,
    Jellyfin,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MediaServerProviderValue {
    Jellyfin,
    Plex,
    Emby,
}

impl MediaServerProviderValue {
    pub fn into_domain(self) -> scryer_domain::MediaServerProvider {
        match self {
            Self::Jellyfin => scryer_domain::MediaServerProvider::Jellyfin,
            Self::Plex => scryer_domain::MediaServerProvider::Plex,
            Self::Emby => scryer_domain::MediaServerProvider::Emby,
        }
    }

    pub fn from_domain(provider: scryer_domain::MediaServerProvider) -> Self {
        match provider {
            scryer_domain::MediaServerProvider::Jellyfin => Self::Jellyfin,
            scryer_domain::MediaServerProvider::Plex => Self::Plex,
            scryer_domain::MediaServerProvider::Emby => Self::Emby,
        }
    }
}

impl ExternalAccountProviderValue {
    pub fn into_domain(self) -> scryer_domain::ExternalAccountProvider {
        match self {
            Self::Plex => scryer_domain::ExternalAccountProvider::Plex,
            Self::Jellyfin => scryer_domain::ExternalAccountProvider::Jellyfin,
        }
    }

    pub fn from_domain(provider: scryer_domain::ExternalAccountProvider) -> Self {
        match provider {
            scryer_domain::ExternalAccountProvider::Plex => Self::Plex,
            scryer_domain::ExternalAccountProvider::Jellyfin => Self::Jellyfin,
        }
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ExternalAccountStatusValue {
    PendingClaim,
    Active,
    Disabled,
}

impl ExternalAccountStatusValue {
    pub fn from_domain(status: scryer_domain::ExternalAccountStatus) -> Self {
        match status {
            scryer_domain::ExternalAccountStatus::PendingClaim => Self::PendingClaim,
            scryer_domain::ExternalAccountStatus::Active => Self::Active,
            scryer_domain::ExternalAccountStatus::Disabled => Self::Disabled,
        }
    }
}

#[derive(InputObject)]
pub struct LoginWithPlexInput {
    pub connection_id: ID,
    pub plex_auth_token: String,
    pub persist_session: Option<bool>,
}

#[derive(InputObject)]
pub struct LoginWithJellyfinInput {
    pub connection_id: ID,
    pub username: String,
    pub password: String,
    pub totp_code: Option<String>,
    pub persist_session: Option<bool>,
}

#[derive(InputObject)]
pub struct WebauthnCompleteInput {
    pub challenge_id: ID,
    pub response_json: Json<serde_json::Value>,
}

#[derive(InputObject)]
pub struct WebauthnRegisterCompleteInput {
    pub challenge_id: ID,
    pub response_json: Json<serde_json::Value>,
    pub friendly_name: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct LoginPayload {
    pub token: String,
    pub user: UserPayload,
    pub expires_at: DateTime<Utc>,
    pub mfa_verified_until: Option<DateTime<Utc>>,
    pub mfa_enrollment_required: bool,
}

#[derive(InputObject)]
pub struct TotpEnrollmentCompleteInput {
    pub challenge_id: ID,
    pub code: String,
}

#[derive(InputObject)]
pub struct TotpVerifyInput {
    pub code: String,
}

#[derive(SimpleObject, Clone)]
pub struct TotpStatusPayload {
    pub enabled: bool,
    pub created_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub recovery_codes_remaining: i32,
}

#[derive(SimpleObject, Clone)]
pub struct TotpEnrollmentStartPayload {
    pub challenge_id: ID,
    pub otpauth_url: String,
    pub secret_base32: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct TotpEnrollmentCompletePayload {
    pub status: TotpStatusPayload,
    pub recovery_codes: Vec<String>,
}

#[derive(SimpleObject, Clone)]
pub struct LoginMfaEnrollmentCompletePayload {
    pub status: TotpStatusPayload,
    pub recovery_codes: Vec<String>,
    pub login: LoginPayload,
}

#[derive(SimpleObject, Clone)]
pub struct WebauthnChallengePayload {
    pub challenge_id: ID,
    pub options_json: Json<serde_json::Value>,
}

#[derive(SimpleObject, Clone)]
pub struct PasskeySummaryPayload {
    pub id: ID,
    pub friendly_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
pub struct DeleteMyPasskeyPayload {
    pub id: ID,
}

#[derive(SimpleObject, Clone)]
pub struct OAuthConnectedAppPayload {
    pub grant_id: ID,
    pub client_id: String,
    pub client_name: String,
    pub authorized_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
pub struct RevokeMyOauthAppPayload {
    pub grant_id: ID,
    /// False when the grant was already revoked (or not owned by the caller)
    /// — access was not newly cut.
    pub revoked: bool,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalIdPayload {
    pub source: String,
    pub value: String,
}

#[derive(SimpleObject, Clone)]
pub struct MediaRequestRequesterPayload {
    pub user_id: ID,
    pub username: String,
    pub avatar_url: Option<String>,
    pub requested_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct MediaRequestPayload {
    pub id: ID,
    pub library_id: ID,
    pub facet: MediaFacetValue,
    pub status: MediaRequestStatusValue,
    pub identity_fingerprint: String,
    pub title: String,
    pub sort_title: Option<String>,
    pub slug: Option<String>,
    pub poster_url: Option<String>,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub language: Option<String>,
    pub content_status: Option<String>,
    pub requested_quality_profile_id: Option<ID>,
    pub requested_quality_profile_name: Option<String>,
    pub requested_monitor_type: Option<MonitorTypeValue>,
    pub resolved_by_user_id: Option<ID>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub created_title_id: Option<ID>,
    pub approved_quality_profile_id: Option<ID>,
    pub approved_quality_profile_name: Option<String>,
    pub external_ids: Vec<ExternalIdPayload>,
    pub requesters: Vec<MediaRequestRequesterPayload>,
    pub created_by_user_id: ID,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct MediaRequestChangedPayload {
    pub event_id: ID,
    pub event_type: DomainEventTypeValue,
    pub request_id: ID,
    pub library_id: ID,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ProviderCatalogFamilyValue {
    Subtitle,
    Notification,
    Indexer,
    DownloadClient,
    ArchiveExtractor,
}

#[derive(SimpleObject, Clone)]
pub struct SubmitMediaRequestPayload {
    /// The submitted (or deduplicated) media request.
    pub request_id: ID,
}

#[derive(SimpleObject, Clone)]
pub struct AudioStreamDetailPayload {
    pub codec: Option<String>,
    pub channels: Option<i32>,
    pub language: Option<String>,
    pub bitrate_kbps: Option<i32>,
}

#[derive(SimpleObject, Clone)]
pub struct SubtitleStreamDetailPayload {
    pub codec: Option<String>,
    pub language: Option<String>,
    pub name: Option<String>,
    pub forced: bool,
    pub default: bool,
}

#[derive(SimpleObject, Clone)]
pub struct SystemHealthPayload {
    pub service_ready: bool,
    pub db_path: String,
    pub datastore_engine: String,
    pub datastore_migration_key: Option<String>,
    pub runtime_path_style: RuntimePathStyleValue,
    pub total_titles: i32,
    pub monitored_titles: i32,
    pub total_users: i32,
    pub titles_movie: i32,
    pub titles_series: i32,
    pub titles_anime: i32,
    pub titles_other: i32,
    pub recent_events: i32,
    pub recent_event_preview: Vec<String>,
    pub db_migration_version: Option<String>,
    pub indexer_stats: Vec<IndexerQueryStatsPayload>,
}

#[derive(SimpleObject, Clone, Debug)]
pub struct RuntimeInfoPayload {
    pub runtime_path_style: RuntimePathStyleValue,
}

#[derive(SimpleObject, Clone)]
pub struct SmgVersionCompatibilityNoticePayload {
    pub status: String,
    pub minimum_version: String,
    pub your_version: String,
    pub message: String,
    pub upgrade_deadline: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct SmgScryerUpdateNoticePayload {
    pub available: bool,
    pub current_version: String,
    pub latest_version: String,
    pub latest_tag: String,
    pub release_url: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub checked_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct IndexerQueryStatsPayload {
    pub indexer_id: ID,
    pub indexer_name: String,
    pub queries_last_24h: i32,
    pub successful_last_24h: i32,
    pub failed_last_24h: i32,
    pub last_query_at: Option<DateTime<Utc>>,
    pub api_current: Option<i32>,
    pub api_max: Option<i32>,
    pub grab_current: Option<i32>,
    pub grab_max: Option<i32>,
}

#[derive(SimpleObject, Clone)]
pub struct UserPayload {
    pub id: ID,
    pub username: String,
    pub login_enabled: bool,
    pub is_default_admin: bool,
    pub has_password: bool,
    pub has_mfa: bool,
    pub has_passkey: bool,
    pub account_kind: UserAccountKindValue,
    pub app_permissions: Vec<AppPermissionValue>,
    pub library_permissions: Vec<UserLibraryPermissionGrantPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct LinkedAccountPayload {
    pub id: ID,
    pub user_id: ID,
    pub provider: ExternalAccountProviderValue,
    pub connection_id: ID,
    pub external_user_id: Option<String>,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub status: ExternalAccountStatusValue,
    pub verified_at: Option<DateTime<Utc>>,
    pub last_login_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct UserLibraryPermissionGrantPayload {
    pub library_id: ID,
    pub permissions: Vec<LibraryPermissionValue>,
}

#[derive(SimpleObject, Clone)]
pub struct EventPayload {
    pub id: ID,
    pub event_type: String,
    pub actor_kind: ActorKindValue,
    pub actor_user_id: Option<ID>,
    pub actor_display_name: String,
    pub title_id: Option<ID>,
    pub message: String,
    pub occurred_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct ActivityEventPayload {
    pub id: ID,
    pub kind: ActivityKindValue,
    pub severity: ActivitySeverityValue,
    pub channels: Vec<ActivityChannelValue>,
    pub actor_kind: ActorKindValue,
    pub actor_user_id: Option<ID>,
    pub actor_display_name: String,
    pub title_id: Option<ID>,
    pub facet: Option<MediaFacetValue>,
    pub message: String,
    pub occurred_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct DomainEventEnvelopePayload {
    pub sequence: Long,
    pub event_id: ID,
    pub occurred_at: DateTime<Utc>,
    pub actor_kind: ActorKindValue,
    pub actor_user_id: Option<ID>,
    pub actor_display_name: String,
    pub title_id: Option<ID>,
    pub facet: Option<MediaFacetValue>,
    pub event_type: DomainEventTypeValue,
    pub stream_kind: StreamKindValue,
    pub stream_id: Option<ID>,
    pub payload_json: Json<serde_json::Value>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum JobKeyValue {
    LibraryScanMovies,
    LibraryScanSeries,
    LibraryScanAnime,
    BackgroundLibraryRefreshMovies,
    BackgroundLibraryRefreshSeries,
    BackgroundLibraryRefreshAnime,
    RssSync,
    SubtitleSearch,
    PluginRegistryRefresh,
    Housekeeping,
    HealthChecks,
    AutoBackup,
    ProwlarrSync,
    PendingReleaseProcessing,
    StagedNzbPrune,
    DiscoverySync,
    TitleImageCacheRefresh,
    TitleDeletion,
    MediaFileDeletion,
    RecycleBinRestore,
    AcquisitionSearch,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum JobCategoryValue {
    Library,
    Acquisition,
    Maintenance,
    Subtitles,
    System,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum JobSectionValue {
    Primary,
    Maintenance,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum JobScheduleKindValue {
    Manual,
    Interval,
    StartupAndInterval,
    DailyAtTime,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum JobTriggerSourceValue {
    Manual,
    ScheduledStartup,
    ScheduledInterval,
    ScheduledDaily,
    SystemInternal,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum JobRunStatusValue {
    Queued,
    Discovering,
    Running,
    Completed,
    Warning,
    Failed,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LibraryScanModeValue {
    Full,
    Additive,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum LibraryScanStatusValue {
    Discovering,
    Running,
    Completed,
    Canceled,
    Warning,
    Failed,
}

#[derive(SimpleObject, Clone)]
pub struct LibraryScanPhaseProgressPayload {
    pub total: i32,
    pub completed: i32,
    pub failed: i32,
}

#[derive(SimpleObject, Clone)]
pub struct LibraryScanProgressPayload {
    pub session_id: ID,
    pub facet: MediaFacetValue,
    pub library_id: Option<ID>,
    pub mode: LibraryScanModeValue,
    pub status: LibraryScanStatusValue,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub found_titles: i32,
    pub title_match_total_known: bool,
    pub title_match_progress: LibraryScanPhaseProgressPayload,
    pub hydration_total_known: bool,
    pub hydration_progress: LibraryScanPhaseProgressPayload,
    pub media_analysis_total_known: bool,
    pub media_analysis_progress: LibraryScanPhaseProgressPayload,
    pub summary: Option<LibraryScanSummaryPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct JobScheduleInfoPayload {
    pub kind: JobScheduleKindValue,
    pub description: String,
    pub interval_seconds: Option<i32>,
    pub initial_delay_seconds: Option<i32>,
    pub next_run_at: Option<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
pub struct JobDefinitionPayload {
    pub key: JobKeyValue,
    pub display_name: String,
    pub description: String,
    pub category: JobCategoryValue,
    pub section: JobSectionValue,
    pub manual_trigger_allowed: bool,
    pub uses_library_scan_progress: bool,
    pub schedule: JobScheduleInfoPayload,
}

#[derive(SimpleObject, Clone)]
pub struct JobRunPayload {
    pub id: ID,
    pub job_key: JobKeyValue,
    pub display_name: String,
    pub category: JobCategoryValue,
    pub section: JobSectionValue,
    pub status: JobRunStatusValue,
    pub trigger_source: JobTriggerSourceValue,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub summary_json: Option<Json<serde_json::Value>>,
    pub summary_text: Option<String>,
    pub error_text: Option<String>,
    pub progress_json: Option<Json<serde_json::Value>>,
    pub library_scan_progress: Option<LibraryScanProgressPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct DiscoverySyncStatusPayload {
    pub state: DiscoverySyncStatePayload,
    pub pending_context_change_count: Long,
}

#[derive(SimpleObject, Clone)]
pub struct DiscoverySyncStatePayload {
    pub last_success_generation_id: Option<ID>,
    pub last_public_feed_generation_id: Option<ID>,
    pub last_context_snapshot_completed_at: Option<DateTime<Utc>>,
    pub last_incremental_reload_completed_at: Option<DateTime<Utc>>,
    pub last_public_feed_completed_at: Option<DateTime<Utc>>,
    pub next_context_snapshot_eligible_at: Option<DateTime<Utc>>,
    pub next_incremental_reload_eligible_at: Option<DateTime<Utc>>,
    pub next_public_feed_eligible_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(InputObject, Clone, Default)]
pub struct DiscoveryHomeInput {
    pub include_public: Option<bool>,
    pub include_personalized: Option<bool>,
    pub include_unresolved: Option<bool>,
    pub limit_per_section: Option<i32>,
    pub filters: Option<DiscoveryHomeFiltersInput>,
}

#[derive(InputObject, Clone, Default)]
pub struct DiscoveryHomeFiltersInput {
    pub content_types: Option<Vec<MediaFacetValue>>,
    pub genre_tag_keys: Option<Vec<String>>,
    pub theme_tag_keys: Option<Vec<String>>,
    pub studio_slugs: Option<Vec<String>>,
    pub minimum_year: Option<i32>,
    pub maximum_year: Option<i32>,
    pub minimum_rating: Option<f64>,
}

#[derive(InputObject, Clone, Default)]
pub struct DiscoveryHomeFilterOptionsInput {
    pub include_public: Option<bool>,
    pub include_personalized: Option<bool>,
    pub include_unresolved: Option<bool>,
}

#[derive(InputObject, Clone, Default)]
pub struct DiscoveryItemsInput {
    pub query: Option<String>,
    pub target_kinds: Option<Vec<String>>,
    pub sources: Option<Vec<String>>,
    pub relation_types: Option<Vec<String>>,
    pub relation_subtypes: Option<Vec<String>>,
    pub genres: Option<Vec<String>>,
    pub status_tags: Option<Vec<String>>,
    pub facet_terms: Option<Vec<String>>,
    pub include_owned: Option<bool>,
    pub include_unresolved: Option<bool>,
    pub include_public: Option<bool>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

#[derive(InputObject, Clone)]
pub struct DiscoveryItemDetailInput {
    pub target_key: String,
    pub include_unresolved: Option<bool>,
}

#[derive(SimpleObject, Clone)]
pub struct DiscoveryHomePayload {
    pub status: DiscoverySyncStatusPayload,
    pub hero_item: Option<DiscoveryItemPayload>,
    pub public_sections: Vec<DiscoverySectionPayload>,
    pub personalized_sections: Vec<DiscoverySectionPayload>,
    pub complete_collection: Option<DiscoverySectionPayload>,
    pub facets: Vec<DiscoveryFacetPayload>,
    pub can_view_personalized: bool,
}

#[derive(SimpleObject, Clone)]
pub struct DiscoveryHomeCardsPayload {
    pub status: DiscoverySyncStatusPayload,
    pub hero_item: Option<DiscoveryHomeHeroPayload>,
    pub public_sections: Vec<DiscoveryHomeSectionPayload>,
    pub personalized_sections: Vec<DiscoveryHomeSectionPayload>,
    pub complete_collection: Option<DiscoveryHomeSectionPayload>,
    pub can_view_personalized: bool,
}

#[derive(SimpleObject, Clone)]
pub struct DiscoveryHomeSectionPayload {
    pub section_id: String,
    pub section_type: String,
    pub title: String,
    pub surface: DiscoverySurfaceValue,
    pub total_count: Long,
    pub items: Vec<DiscoveryHomeCardPayload>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DiscoverySurfaceValue {
    Public,
    Personalized,
    Mixed,
}

#[derive(SimpleObject, Clone)]
pub struct DiscoveryHomeCardPayload {
    pub id: ID,
    pub target_key: String,
    pub target_kind: MediaFacetValue,
    pub display_title: String,
    pub original_title: Option<String>,
    pub sort_title: Option<String>,
    pub year: Option<i32>,
    pub poster_url: Option<String>,
    pub content_type: MediaFacetValue,
    pub owned_in_input: bool,
}

#[derive(SimpleObject, Clone)]
pub struct DiscoveryHomeHeroPayload {
    pub id: ID,
    pub target_key: String,
    pub target_kind: MediaFacetValue,
    pub display_title: String,
    pub original_title: Option<String>,
    pub sort_title: Option<String>,
    pub year: Option<i32>,
    pub poster_url: Option<String>,
    pub background_url: Option<String>,
    pub overview: Option<String>,
    pub content_type: MediaFacetValue,
    pub rating: Option<f64>,
    pub rating_sources: Vec<String>,
    pub external_ratings: Vec<DiscoveryExternalRatingPayload>,
    pub genre_tags: Vec<CanonicalMediaTagPayload>,
    pub matched_subject_count: i32,
    pub owned_in_input: bool,
}

#[derive(SimpleObject, Clone)]
pub struct DiscoveryHomeFilterOptionsPayload {
    pub genres: Vec<CanonicalTagFilterOptionPayload>,
    pub themes: Vec<CanonicalTagFilterOptionPayload>,
    pub studio_slugs: Vec<String>,
}

#[derive(SimpleObject, Clone)]
pub struct CanonicalTagFilterOptionPayload {
    pub key: String,
    pub name: String,
}

#[derive(SimpleObject, Clone)]
pub struct DiscoveryItemsPayload {
    pub items: Vec<DiscoveryItemPayload>,
    pub total_count: Long,
    pub can_view_personalized: bool,
}

#[derive(InputObject, Clone)]
pub struct CatalogDiscoveryInput {
    pub facet: MediaFacetValue,
    pub library_ids: Option<Vec<ID>>,
    pub include_unresolved: Option<bool>,
    pub limit_per_group: Option<i32>,
    pub max_groups: Option<i32>,
}

#[derive(SimpleObject, Clone)]
pub struct CatalogDiscoveryPayload {
    pub can_view_personalized: bool,
    pub groups: Vec<CatalogDiscoveryGroupPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct CatalogDiscoveryGroupPayload {
    pub id: String,
    pub kind: CatalogDiscoveryGroupKindValue,
    pub surface: CatalogDiscoverySurfaceValue,
    pub label_value: Option<String>,
    pub total_count: Long,
    pub items: Vec<DiscoveryItemPayload>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum CatalogDiscoveryGroupKindValue {
    PublicTop,
    PublicSection,
    GenreAffinity,
    ThemeAffinity,
    Acclaimed,
    CompleteCollection,
    Fallback,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum CatalogDiscoverySurfaceValue {
    Public,
    Personalized,
}

#[derive(SimpleObject, Clone)]
pub struct DiscoverySectionPayload {
    pub section_id: String,
    pub section_type: String,
    pub title: String,
    pub surface: String,
    pub total_count: Long,
    pub items: Vec<DiscoveryItemPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct DiscoveryExternalRatingPayload {
    pub source: String,
    pub value: Option<f64>,
    pub score: Option<f64>,
    pub normalized: f64,
    pub votes: Option<i32>,
    pub url: String,
}

#[derive(SimpleObject, Clone)]
pub struct DiscoveryExternalIdPayload {
    pub source: String,
    pub kind: String,
    pub id: String,
    pub key: String,
}

#[derive(SimpleObject, Clone)]
pub struct CanonicalMediaTagPayload {
    pub key: String,
    pub category: String,
    pub name: String,
    pub confidence: Option<f64>,
    pub sources: Vec<String>,
    pub source_tag_keys: Vec<String>,
    pub is_adult: bool,
    pub is_spoiler: bool,
}

#[derive(SimpleObject, Clone)]
pub struct DiscoveryItemPayload {
    pub id: ID,
    pub target_key: String,
    pub target_kind: String,
    pub resolved: bool,
    pub resolved_title_id: Option<ID>,
    pub display_title: String,
    pub original_title: Option<String>,
    pub sort_title: Option<String>,
    pub year: Option<i32>,
    pub poster_url: Option<String>,
    pub background_url: Option<String>,
    pub overview: Option<String>,
    pub content_type: Option<String>,
    pub canonical_tags: Vec<CanonicalMediaTagPayload>,
    pub rating: Option<f64>,
    pub rating_sources: Vec<String>,
    pub external_ratings: Vec<DiscoveryExternalRatingPayload>,
    pub external_ids: Vec<DiscoveryExternalIdPayload>,
    pub status_tags: Vec<String>,
    pub source_tags: Vec<String>,
    pub sources: Vec<String>,
    pub best_source: Option<String>,
    pub relation_types: Vec<String>,
    pub relation_subtypes: Vec<String>,
    pub source_count: Option<i32>,
    pub edge_count: Option<i32>,
    pub relation_count: Option<i32>,
    pub source_subject_count: Option<i32>,
    pub rank_score: Option<f64>,
    pub matched_subject_titles: Vec<String>,
    pub matched_subject_count: i32,
    pub tmdb_collection_id: Option<String>,
    pub tmdb_collection_name: Option<String>,
    pub owned_in_input: bool,
    pub studio_slug: Option<String>,
    pub person_ids: Vec<i32>,
    pub facet_terms: Vec<String>,
    pub context_terms: Vec<String>,
}

#[derive(SimpleObject, Clone)]
pub struct DiscoveryFacetPayload {
    pub name: String,
    pub value: String,
    pub smg_count: Option<Long>,
    pub local_count: Option<Long>,
}

#[derive(SimpleObject, Clone)]
pub struct TitleReleaseBlocklistEntryPayload {
    pub id: ID,
    pub source_hint: Option<String>,
    pub source_title: Option<String>,
    pub error_message: Option<String>,
    pub attempted_at: DateTime<Utc>,
    pub episode_ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
pub struct IndexerSearchResultPayload {
    pub source: String,
    pub title: String,
    pub link: Option<String>,
    pub download_url: Option<String>,
    pub source_kind: Option<DownloadSourceKindValue>,
    pub size_bytes: Option<Long>,
    pub published_at: Option<DateTime<Utc>>,
    pub thumbs_up: Option<i32>,
    pub thumbs_down: Option<i32>,
    pub parsed_release: Option<ParsedReleasePayload>,
    pub quality_profile_decision: Option<QualityProfileDecisionPayload>,
    // Torrent-specific fields
    pub seeders: Option<i32>,
    pub peers: Option<i32>,
    pub info_hash: Option<String>,
    pub freeleech: Option<bool>,
    pub download_volume_factor: Option<f64>,
    pub candidate_token: Option<String>,
    pub queue_scope: Option<QueueDownloadScopePayload>,
    pub auto_eligible: Option<bool>,
    pub auto_decision_code: Option<String>,
    pub auto_decision_summary: Option<String>,
}

/// The acquisition scope a queued download targets, as a real union: clients
/// branch on `__typename` instead of a string discriminator.
#[derive(async_graphql::Union, Clone)]
pub enum QueueDownloadScopePayload {
    Episode(EpisodeScopePayload),
    EpisodeSet(EpisodeSetScopePayload),
    SeriesMovie(SeriesMovieScopePayload),
    Collection(CollectionScopePayload),
    Title(TitleScopePayload),
    Orphan(OrphanScopePayload),
}

impl QueueDownloadScopePayload {
    pub fn episode(episode_id: ID) -> Self {
        Self::Episode(EpisodeScopePayload { episode_id })
    }
}

#[derive(SimpleObject, Clone)]
pub struct EpisodeScopePayload {
    pub episode_id: ID,
}

#[derive(SimpleObject, Clone)]
pub struct EpisodeSetScopePayload {
    pub episode_ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
pub struct SeriesMovieScopePayload {
    pub series_movie_link_id: ID,
}

#[derive(SimpleObject, Clone)]
pub struct CollectionScopePayload {
    pub collection_id: ID,
}

/// Marker member: the scope is the whole title.
#[derive(SimpleObject, Clone)]
pub struct TitleScopePayload {
    pub whole_title: bool,
}

/// Marker member: the download is not attached to any known scope.
#[derive(SimpleObject, Clone)]
pub struct OrphanScopePayload {
    pub orphaned: bool,
}

#[derive(SimpleObject, Clone)]
pub struct ParsedEpisodePayload {
    pub season: Option<i32>,
    pub episode_numbers: Vec<i32>,
}

#[derive(SimpleObject, Clone)]
pub struct ParsedReleasePayload {
    pub raw_title: String,
    pub normalized_title: String,
    pub release_group: Option<String>,
    pub quality: Option<String>,
    pub source: Option<String>,
    pub video_codec: Option<String>,
    pub video_encoding: Option<String>,
    pub audio: Option<String>,
    pub is_dual_audio: bool,
    pub is_atmos: bool,
    pub is_dolby_vision: bool,
    pub detected_hdr: bool,
    pub is_proper_upload: bool,
    pub is_remux: bool,
    pub is_bd_disk: bool,
    pub is_ai_enhanced: bool,
    pub parse_confidence: f32,
    pub parse_hints: Vec<String>,
    pub episode: Option<ParsedEpisodePayload>,
}

#[derive(SimpleObject, Clone)]
pub struct ScoringEntryPayload {
    pub code: String,
    pub delta: i32,
    pub source: String,
    pub rule_set_name: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct QualityProfileDecisionPayload {
    pub allowed: bool,
    pub block_codes: Vec<String>,
    pub release_score: i32,
    pub preference_score: i32,
    pub scoring_log: Vec<ScoringEntryPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct ProviderConfigValuePayload {
    pub key: String,
    pub label: Option<String>,
    pub field_type: Option<PluginConfigFieldTypeValue>,
    pub required: bool,
    pub default_value: Option<String>,
    pub value_source: Option<PluginConfigValueSourceValue>,
    pub role: Option<PluginConfigFieldRoleValue>,
    pub host_binding: Option<String>,
    pub options: Vec<PluginConfigFieldOptionPayload>,
    pub help_text: Option<String>,
    /// The stored value as a typed union; null when the field is unset.
    pub value: Option<ProviderConfigFieldValue>,
}

/// A provider-config field's stored value: clients branch on `__typename`
/// instead of probing a nullable per-type fan-out.
#[derive(async_graphql::Union, Clone)]
pub enum ProviderConfigFieldValue {
    String(StringConfigValuePayload),
    Bool(BoolConfigValuePayload),
    Int(IntConfigValuePayload),
    Float(FloatConfigValuePayload),
    Secret(SecretConfigValuePayload),
}

#[derive(SimpleObject, Clone)]
pub struct StringConfigValuePayload {
    pub value: String,
}

#[derive(SimpleObject, Clone)]
pub struct BoolConfigValuePayload {
    pub value: bool,
}

#[derive(SimpleObject, Clone)]
pub struct IntConfigValuePayload {
    pub value: i64,
}

#[derive(SimpleObject, Clone)]
pub struct FloatConfigValuePayload {
    pub value: f64,
}

/// Secret fields never echo their value; `stored` reports presence.
#[derive(SimpleObject, Clone)]
pub struct SecretConfigValuePayload {
    pub stored: bool,
}

#[derive(InputObject, Clone)]
pub struct ProviderConfigValueInput {
    pub key: String,
    pub string_value: Option<String>,
    pub bool_value: Option<bool>,
    pub int_value: Option<i64>,
    pub float_value: Option<f64>,
    pub secret_value: Option<String>,
    pub clear_secret: Option<bool>,
}

#[derive(SimpleObject, Clone)]
pub struct IndexerConfigPayload {
    pub id: ID,
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    pub indexer_proxy_config_id: Option<ID>,
    pub has_api_key: bool,
    pub is_managed: bool,
    pub managed_parent_config_id: Option<ID>,
    pub supports_managed_children_sync: bool,
    pub stored_secret_keys: Vec<String>,
    pub rate_limit_seconds: Option<i64>,
    pub rate_limit_burst: Option<i64>,
    pub disabled_until: Option<DateTime<Utc>>,
    pub is_enabled: bool,
    pub enable_interactive_search: bool,
    pub enable_auto_search: bool,
    pub last_health_status: Option<String>,
    pub last_error_at: Option<DateTime<Utc>>,
    pub last_query_at: Option<DateTime<Utc>>,
    pub config: Vec<ProviderConfigValuePayload>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct IndexerProxyConfigPayload {
    pub id: ID,
    pub name: String,
    pub provider_type: String,
    pub protocol: String,
    pub base_url: String,
    pub request_timeout_seconds: i32,
    pub is_enabled: bool,
    pub last_health_status: Option<String>,
    pub last_error_message: Option<String>,
    pub last_error_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct IndexerProxyTestResultPayload {
    pub ok: bool,
    pub status: String,
    pub message: Option<String>,
    pub duration_ms: Option<i32>,
}

#[derive(SimpleObject, Clone)]
pub struct IndexerConfigSyncPayload {
    pub parent_config_id: ID,
    pub created_ids: Vec<ID>,
    pub updated_ids: Vec<ID>,
    pub deleted_ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
pub struct RootFolderPayload {
    pub path: String,
    pub is_default: bool,
}

#[derive(SimpleObject, Clone)]
pub struct LibraryRootPayload {
    pub id: ID,
    pub path: String,
    pub is_default: bool,
}

#[derive(SimpleObject, Clone)]
pub struct LibrarySettingsPayload {
    pub required_audio_languages_override: Option<Vec<String>>,
    pub required_audio_languages: Vec<String>,
    pub quality_profile_id_override: Option<ID>,
    pub quality_profile_id: ID,
    pub request_quality_profile_ids_override: Option<Vec<ID>>,
    pub request_quality_profile_ids: Vec<ID>,
    pub request_quality_profile_default_id: ID,
    pub scoring_persona_override: Option<ScoringPersonaValue>,
    pub scoring_persona: ScoringPersonaValue,
    pub filler_policy_override: Option<FillerPolicyValue>,
    pub filler_policy: Option<FillerPolicyValue>,
    pub recap_policy_override: Option<RecapPolicyValue>,
    pub recap_policy: Option<RecapPolicyValue>,
    pub monitor_specials_override: Option<bool>,
    pub monitor_specials: Option<bool>,
    pub inter_season_movies_override: Option<bool>,
    pub inter_season_movies: Option<bool>,
    pub monitor_filler_movies_override: Option<bool>,
    pub monitor_filler_movies: Option<bool>,
    pub nfo_write_on_import_override: Option<bool>,
    pub nfo_write_on_import: bool,
    pub plexmatch_write_on_import_override: Option<bool>,
    pub plexmatch_write_on_import: Option<bool>,
    pub import_mode_override: Option<ImportModeValue>,
    pub import_mode: ImportModeValue,
    pub set_permissions_linux_override: Option<bool>,
    pub set_permissions_linux: bool,
    pub file_chmod_override: Option<String>,
    pub file_chmod: Option<String>,
    pub folder_chmod_override: Option<String>,
    pub folder_chmod: Option<String>,
    pub chown_group_override: Option<String>,
    pub chown_group: Option<String>,
    pub indexer_routing_override: Option<Vec<IndexerRoutingEntryPayload>>,
    pub download_client_routing_override: Option<Vec<DownloadClientRoutingEntryPayload>>,
}

#[derive(SimpleObject, Clone)]
pub struct DownloadClientConfigPayload {
    pub id: ID,
    pub name: String,
    pub client_type: String,
    pub base_url: Option<String>,
    pub config: Vec<ProviderConfigValuePayload>,
    pub stored_secret_keys: Vec<String>,
    pub is_enabled: bool,
    pub status: String,
    pub last_error: Option<String>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct SubtitleProviderConfigPayload {
    pub id: ID,
    pub name: String,
    pub provider_type: String,
    pub has_config: bool,
    pub stored_secret_keys: Vec<String>,
    pub enabled_facets: Vec<MediaFacetValue>,
    pub is_enabled: bool,
    pub last_health_status: Option<String>,
    pub last_error: Option<String>,
    pub last_error_at: Option<DateTime<Utc>>,
    pub disabled_until: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct DownloadClientFilterOptionPayload {
    pub client_id: ID,
    pub client_name: String,
    pub client_type: String,
}

#[derive(SimpleObject, Clone)]
pub struct ImportResultPayload {
    pub import_id: ID,
    pub decision: ImportDecisionValue,
    pub skip_reason: Option<ImportSkipReasonValue>,
    pub title_id: Option<ID>,
    pub source_path: String,
    pub dest_path: Option<String>,
    pub error_message: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct ImportRecordPayload {
    pub id: ID,
    pub source_system: String,
    pub source_ref: String,
    pub source_title: Option<String>,
    pub facet: Option<MediaFacetValue>,
    pub import_type: ImportTypeValue,
    pub status: ImportStatusValue,
    pub error_message: Option<String>,
    pub decision: Option<ImportDecisionValue>,
    pub skip_reason: Option<ImportSkipReasonValue>,
    pub title_id: Option<ID>,
    pub source_path: Option<String>,
    pub dest_path: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(InputObject)]
pub struct RetryImportInput {
    pub import_id: ID,
    pub password: Option<String>,
}

#[derive(InputObject)]
pub struct IgnoreTrackedDownloadInput {
    pub client_id: Option<ID>,
    pub client_type: String,
    pub download_client_item_id: String,
}

#[derive(InputObject)]
pub struct MarkTrackedDownloadFailedInput {
    pub client_id: Option<ID>,
    pub client_type: String,
    pub download_client_item_id: String,
    pub skip_reacquire: Option<bool>,
}

#[derive(OneofObject, Clone)]
pub enum QueueDownloadScopeInput {
    Episode(ID),
    EpisodeSet(Vec<ID>),
    SeriesMovie(ID),
    Collection(ID),
    Title(bool),
}

#[derive(InputObject)]
pub struct AssignTrackedDownloadTitleInput {
    pub client_id: Option<ID>,
    pub client_type: String,
    pub download_client_item_id: String,
    pub title_id: ID,
    pub scope: QueueDownloadScopeInput,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum AddTitleHydrationStateValue {
    Pending,
    Complete,
    NotRequired,
}

#[derive(InputObject)]
pub struct ScanLibraryInput {
    pub library_id: ID,
    pub import_warmup_session_id: Option<ID>,
}

#[derive(SimpleObject, Clone)]
pub struct LibraryScanSummaryPayload {
    pub scanned: i32,
    pub matched: i32,
    pub imported: i32,
    pub skipped: i32,
    pub unmatched: i32,
}

#[derive(SimpleObject, Clone)]
pub struct PendingImportCountsPayload {
    pub movie: i32,
    pub series: i32,
    pub anime: i32,
}

#[derive(SimpleObject, Clone)]
pub struct MediaRequestCountsPayload {
    pub movie: i32,
    pub series: i32,
    pub anime: i32,
}

#[derive(SimpleObject, Clone)]
pub struct NavigationBadgeCountsPayload {
    pub pending_import_counts: PendingImportCountsPayload,
    pub pending_media_request_counts: MediaRequestCountsPayload,
    pub activity_import_count: i32,
    pub plugin_update_count: i32,
}

#[derive(SimpleObject, Clone)]
pub struct PendingImportSearchAttemptPayload {
    pub query: String,
    pub result_count: i32,
    pub top_results: Vec<String>,
    pub summary: String,
}

#[derive(SimpleObject, Clone)]
pub struct PendingImportItemPayload {
    pub id: ID,
    pub library_id: ID,
    pub facet: MediaFacetValue,
    pub status: PendingImportStatusValue,
    pub title_id: Option<ID>,
    pub title_name: Option<String>,
    pub title_slug: Option<String>,
    pub display_name: String,
    pub path: String,
    pub folder_path: Option<String>,
    pub query: String,
    pub year_hint: Option<i32>,
    pub reason: String,
    pub search_attempts: Vec<PendingImportSearchAttemptPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct PendingImportConnectionPayload {
    pub items: Vec<PendingImportItemPayload>,
    pub total_count: i32,
    pub has_more: bool,
}

#[derive(InputObject)]
pub struct ResolvePendingImportInput {
    pub pending_import_id: ID,
    pub title: AddTitleInput,
}

#[derive(SimpleObject, Clone)]
pub struct PendingImportBindingFilePreviewPayload {
    pub file_path: String,
    pub file_name: String,
    pub size_bytes: Long,
    pub parsed_season: Option<i32>,
    pub parsed_episodes: Vec<i32>,
    pub parsed_absolute_numbers: Vec<i32>,
    pub suggested_episode_ids: Vec<ID>,
}

#[derive(InputObject)]
pub struct BindPendingImportInput {
    pub pending_import_id: ID,
    pub collection_id: Option<ID>,
    pub episode_ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
pub struct IgnorePendingImportPayload {
    pub id: ID,
    pub status: PendingImportStatusValue,
}

#[derive(SimpleObject, Clone)]
pub struct CancelAcquisitionSearchPayload {
    pub id: ID,
    /// False when the search run had already finished — not an error.
    pub accepted: bool,
}

#[derive(SimpleObject, Clone)]
pub struct CancelLibraryScanPayload {
    pub session_id: ID,
    /// Whether the cancel was accepted; false when the scan had already
    /// finished — not an error.
    pub accepted: bool,
}

#[derive(SimpleObject, Clone)]
pub struct DeletePreviewPayload {
    pub fingerprint: String,
    pub total_file_count: i32,
    pub media_count: i32,
    pub subtitle_count: i32,
    pub image_count: i32,
    pub other_count: i32,
    pub directory_count: i32,
    pub requires_typed_confirmation: bool,
    pub typed_confirmation_prompt: Option<String>,
    pub target_label: String,
    pub sample_paths: Vec<String>,
}

#[derive(SimpleObject, Clone)]
pub struct DeleteTitlePreviewResultPayload {
    pub title_id: ID,
    pub preview: Option<DeletePreviewPayload>,
    pub error: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct DeleteTitlesPreviewPayload {
    pub preview: DeletePreviewPayload,
    pub items: Vec<DeleteTitlePreviewResultPayload>,
    pub failed_count: i32,
}

#[derive(SimpleObject, Clone)]
pub struct DeleteTitlesPayload {
    pub job_run: JobRunPayload,
    pub accepted_title_ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
pub struct MediaRenamePlanItemPayload {
    pub collection_id: Option<ID>,
    pub series_movie_link_ids: Vec<ID>,
    pub current_path: String,
    pub proposed_path: Option<String>,
    pub normalized_filename: Option<String>,
    pub collision: bool,
    pub reason_code: String,
    pub write_action: String,
    pub source_size_bytes: Option<Long>,
    pub source_mtime_unix_ms: Option<Long>,
}

#[derive(SimpleObject, Clone)]
pub struct MediaRenamePlanPayload {
    pub facet: MediaFacetValue,
    pub title_id: Option<ID>,
    pub template: String,
    pub collision_policy: RenameCollisionPolicyValue,
    pub missing_metadata_policy: RenameMissingMetadataPolicyValue,
    pub fingerprint: String,
    pub total: i32,
    pub renamable: i32,
    pub noop: i32,
    pub conflicts: i32,
    pub errors: i32,
    pub items: Vec<MediaRenamePlanItemPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct MediaRenameApplyItemPayload {
    pub collection_id: Option<ID>,
    pub series_movie_link_ids: Vec<ID>,
    pub current_path: String,
    pub proposed_path: Option<String>,
    pub final_path: Option<String>,
    pub write_action: String,
    pub status: String,
    pub reason_code: String,
    pub error_message: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct MediaRenameApplyPayload {
    pub plan_fingerprint: String,
    pub total: i32,
    pub applied: i32,
    pub skipped: i32,
    pub failed: i32,
    pub items: Vec<MediaRenameApplyItemPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct SubtitleLanguagePreferencePayload {
    pub code: String,
    pub hearing_impaired: bool,
    pub forced: bool,
}

#[derive(SimpleObject, Clone)]
pub struct SubtitleSettingsPayload {
    pub enabled: bool,
    pub languages: Vec<SubtitleLanguagePreferencePayload>,
    pub auto_download_on_import: bool,
    pub minimum_score_series: i32,
    pub minimum_score_movie: i32,
    pub search_interval_hours: i32,
    pub include_ai_translated: bool,
    pub include_machine_translated: bool,
    pub sync_enabled: bool,
    pub sync_threshold_series: i32,
    pub sync_threshold_movie: i32,
    pub sync_max_offset_seconds: i32,
}

#[derive(SimpleObject, Clone)]
pub struct RecycleBinSettingsPayload {
    pub enabled: bool,
}

#[derive(SimpleObject, Clone)]
pub struct AcquisitionSettingsPayload {
    pub enabled: bool,
    pub upgrade_cooldown_hours: i32,
    pub same_tier_min_delta: i32,
    pub cross_tier_min_delta: i32,
    pub forced_upgrade_delta_bypass: i32,
    pub poll_interval_seconds: i32,
    pub long_tail_backfill_max_scopes_per_cycle: i32,
    pub long_tail_reconverge_days: i32,
}

#[derive(SimpleObject, Clone)]
pub struct PluginHttpTrustedCertificatePayload {
    pub fingerprint_sha256: String,
    pub pem: String,
}

#[derive(SimpleObject, Clone)]
pub struct GeneralSettingsPayload {
    pub keep_history_forever: bool,
    pub history_retention_days: i32,
    pub image_cache_max_size_mb: i32,
    pub effective_image_cache_max_size_bytes: Long,
    pub effective_image_cache_max_size_mb: f64,
    pub image_cache_max_size_env_override_active: bool,
    pub plugin_http_ca_bundle_pem: String,
    pub plugin_http_trusted_certificates: Vec<PluginHttpTrustedCertificatePayload>,
}

#[derive(SimpleObject, Clone)]
pub struct AutoBackupSettingsPayload {
    pub enabled: bool,
    pub daily_time_local: String,
    pub auto_backup_key_present: bool,
    pub auto_backup_disabled_missing_key_notice: bool,
    pub next_run_at: Option<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
pub struct BackupSettingsPayload {
    pub custom_backup_path: Option<String>,
    pub default_backup_path: String,
    pub effective_backup_path: String,
}

#[derive(SimpleObject, Clone)]
pub struct SecuritySettingsPayload {
    pub form_login_enabled: bool,
    pub password_min_length: i32,
    pub skip_login_for_local_ips: bool,
    pub mfa_require_config_step_up: bool,
    pub mfa_require_password_login: bool,
    pub totp_require_jellyfin_login: bool,
    pub effective_form_login_enabled: bool,
    pub env_override_active: bool,
    pub env_override_description: Option<String>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum UiThemeValue {
    Light,
    Dark,
    Pride,
    System,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum UiDateTimeFormatValue {
    Locale,
    #[graphql(name = "ISO24H")]
    Iso24h,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum UiDensityValue {
    Compact,
    Comfortable,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum UiSidebarModeValue {
    Collapsed,
    Expanded,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum UiDefaultLandingViewValue {
    Movies,
    Series,
    Anime,
    Activity,
    Calendar,
    Wanted,
    History,
    Settings,
    System,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum UiSettingsFacetValue {
    Movies,
    Series,
    Anime,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum UiTableViewModeValue {
    Compact,
    PosterTable,
}

#[derive(SimpleObject, Clone)]
pub struct UiTableColumnSettingPayload {
    pub facet: UiSettingsFacetValue,
    pub table_view_mode: UiTableViewModeValue,
    pub column_id: String,
    pub column_order: i32,
    pub visible: bool,
}

#[derive(SimpleObject, Clone)]
pub struct UiSettingsPayload {
    pub theme: UiThemeValue,
    pub date_time_format: UiDateTimeFormatValue,
    pub highlight_color: Option<String>,
    pub secondary_color: Option<String>,
    pub high_contrast_mode: bool,
    pub reduce_motion: bool,
    pub hide_sponsor_button: bool,
    pub density: UiDensityValue,
    pub sidebar_mode: UiSidebarModeValue,
    pub default_landing_view: UiDefaultLandingViewValue,
    pub table_columns: Vec<UiTableColumnSettingPayload>,
}

#[derive(InputObject, Clone)]
pub struct UiTableColumnSettingInput {
    pub facet: UiSettingsFacetValue,
    pub table_view_mode: UiTableViewModeValue,
    pub column_id: String,
    pub column_order: i32,
    pub visible: bool,
}

#[derive(InputObject, Clone)]
pub struct SetMyUiSettingsInput {
    pub theme: UiThemeValue,
    pub date_time_format: Option<UiDateTimeFormatValue>,
    pub highlight_color: Option<String>,
    pub secondary_color: Option<String>,
    pub high_contrast_mode: bool,
    pub reduce_motion: bool,
    pub hide_sponsor_button: bool,
    pub density: UiDensityValue,
    pub sidebar_mode: UiSidebarModeValue,
    pub default_landing_view: UiDefaultLandingViewValue,
    pub table_columns: Vec<UiTableColumnSettingInput>,
}

#[derive(SimpleObject, Clone)]
pub struct AuthRuntimeStatePayload {
    pub effective_form_login_enabled: bool,
    pub skip_login_for_local_ips: bool,
    pub passkey_enabled: bool,
    pub env_override_active: bool,
    pub mfa_require_password_login: bool,
    pub mfa_require_config_step_up: bool,
    pub totp_require_jellyfin_login: bool,
}

#[derive(SimpleObject, Clone)]
pub struct DelayProfilePayload {
    pub id: ID,
    pub name: String,
    pub usenet_delay_minutes: i32,
    pub torrent_delay_minutes: i32,
    pub preferred_protocol: DelayProfilePreferredProtocolValue,
    pub min_age_minutes: i32,
    pub bypass_score_threshold: Option<i32>,
    pub applies_to_facets: Vec<MediaFacetValue>,
    pub tags: Vec<String>,
    pub priority: i32,
    pub enabled: bool,
}

#[derive(SimpleObject, Clone)]
pub struct DelayProfileDeletionPayload {
    pub id: ID,
}

#[derive(SimpleObject, Clone)]
pub struct ScoringOverridesPayload {
    pub allow_x265_non4k: Option<bool>,
    pub block_dv_without_fallback: Option<bool>,
    pub prefer_compact_encodes: Option<bool>,
    pub prefer_lossless_audio: Option<bool>,
    pub block_upscaled: Option<bool>,
}

#[derive(SimpleObject, Clone)]
pub struct QualityProfileCriteriaPayload {
    pub quality_tiers: Vec<String>,
    pub archival_quality: Option<String>,
    pub allow_unknown_quality: bool,
    pub source_allowlist: Vec<String>,
    pub source_blocklist: Vec<String>,
    pub video_codec_allowlist: Vec<String>,
    pub video_codec_blocklist: Vec<String>,
    pub audio_codec_allowlist: Vec<String>,
    pub audio_codec_blocklist: Vec<String>,
    pub dolby_vision_allowed: bool,
    pub detected_hdr_allowed: bool,
    pub prefer_remux: bool,
    pub allow_bd_disk: bool,
    pub allow_upgrades: bool,
    pub scoring_overrides: ScoringOverridesPayload,
    pub cutoff_tier: Option<String>,
    pub min_score_to_grab: Option<i32>,
}

#[derive(SimpleObject, Clone)]
pub struct QualityProfilePayload {
    pub id: ID,
    pub name: String,
    pub criteria: QualityProfileCriteriaPayload,
}

#[derive(SimpleObject, Clone)]
pub struct QualityProfileSelectionPayload {
    pub scope: ContentScopeValue,
    pub override_profile_id: Option<ID>,
    pub effective_profile_id: ID,
    pub inherits_global: bool,
}

#[derive(SimpleObject, Clone)]
pub struct FacetScoringPersonaSelectionPayload {
    pub scope: ContentScopeValue,
    pub override_persona: Option<ScoringPersonaValue>,
    pub effective_persona: ScoringPersonaValue,
    pub inherits_global: bool,
}

#[derive(SimpleObject, Clone)]
pub struct QualityProfileSettingsPayload {
    pub profiles: Vec<QualityProfilePayload>,
    pub global_profile_id: ID,
    pub global_scoring_persona: ScoringPersonaValue,
    pub category_selections: Vec<QualityProfileSelectionPayload>,
    pub category_persona_selections: Vec<FacetScoringPersonaSelectionPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct DownloadClientRoutingEntryPayload {
    pub client_id: ID,
    pub enabled: bool,
    pub category: Option<String>,
    pub recent_queue_priority: Option<String>,
    pub older_queue_priority: Option<String>,
    pub remove_completed: bool,
    pub remove_failed: bool,
}

#[derive(SimpleObject, Clone)]
pub struct IndexerRoutingEntryPayload {
    pub indexer_id: ID,
    pub enabled: bool,
    pub categories: Vec<String>,
    pub priority: i32,
}

#[derive(SimpleObject, Clone)]
pub struct MediaSettingsPayload {
    pub scope: ContentScopeValue,
    pub library_path: String,
    pub root_folders: Vec<RootFolderPayload>,
    pub required_audio_languages: Vec<String>,
    pub folder_template: String,
    pub season_folder_template: Option<String>,
    pub specials_folder_template: Option<String>,
    pub rename_enabled: bool,
    pub rename_template: String,
    pub rename_collision_policy: RenameCollisionPolicyValue,
    pub rename_missing_metadata_policy: RenameMissingMetadataPolicyValue,
    pub filler_policy: Option<FillerPolicyValue>,
    pub recap_policy: Option<RecapPolicyValue>,
    pub monitor_specials: Option<bool>,
    pub inter_season_movies: Option<bool>,
    pub monitor_filler_movies: Option<bool>,
    pub nfo_write_on_import: bool,
    pub plexmatch_write_on_import: Option<bool>,
    pub import_mode: ImportModeValue,
    pub set_permissions_linux: bool,
    pub file_chmod: Option<String>,
    pub folder_chmod: Option<String>,
    pub chown_group: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct LibraryPathsPayload {
    pub movie_path: String,
    pub series_path: String,
    pub anime_path: String,
}

#[derive(SimpleObject, Clone)]
pub struct ServiceSettingsPayload {
    pub tls_cert_path: String,
    pub tls_key_path: String,
}

#[derive(InputObject, Clone)]
pub struct ExternalIdInput {
    pub source: String,
    pub value: String,
}

#[derive(InputObject, Clone)]
pub struct TitleOptionsInput {
    pub quality_profile_id: Option<ID>,
    pub root_folder_id: MaybeUndefined<ID>,
    pub monitor_type: Option<MonitorTypeValue>,
    pub use_season_folders: Option<bool>,
    pub monitor_specials: Option<bool>,
    pub inter_season_movies: Option<bool>,
    pub filler_policy: Option<FillerPolicyValue>,
    pub recap_policy: Option<RecapPolicyValue>,
}

#[derive(InputObject, Clone)]
pub struct AddTitleInput {
    pub name: String,
    pub facet: MediaFacetValue,
    pub library_id: Option<ID>,
    pub monitored: bool,
    pub tags: Vec<String>,
    pub options: Option<TitleOptionsInput>,
    pub external_ids: Option<Vec<ExternalIdInput>>,
    pub source_hint: Option<String>,
    pub source_kind: Option<DownloadSourceKindValue>,
    pub source_title: Option<String>,
    pub min_availability: Option<String>,
    // Non-artwork metadata fields the frontend can supply from the search result.
    // Poster and fanart URLs are sourced from server-side SMG metadata.
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub sort_title: Option<String>,
    pub slug: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub language: Option<String>,
    pub content_status: Option<String>,
}

#[derive(InputObject, Clone)]
pub struct SubmitMediaRequestInput {
    pub library_id: ID,
    pub facet: MediaFacetValue,
    pub title: String,
    pub external_ids: Vec<ExternalIdInput>,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub sort_title: Option<String>,
    pub slug: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub language: Option<String>,
    pub content_status: Option<String>,
    pub requested_quality_profile_id: Option<ID>,
    pub requested_monitor_type: Option<MonitorTypeValue>,
}

#[derive(InputObject, Clone)]
pub struct ApproveMediaRequestInput {
    pub request_id: ID,
    pub quality_profile_id: ID,
    pub monitor_type: Option<MonitorTypeValue>,
}

#[derive(InputObject, Clone)]
pub struct UpdateMediaRequestInput {
    pub request_id: ID,
    pub requested_quality_profile_id: ID,
    pub requested_monitor_type: Option<MonitorTypeValue>,
}

#[derive(SimpleObject, Clone)]
pub struct MediaRequestActionPayload {
    /// The media request the action applied to.
    pub request_id: ID,
}

#[derive(SimpleObject, Clone)]
pub struct ApproveMediaRequestPayload {
    pub title_id: ID,
    pub wanted_search: Option<WantedSearchPayload>,
    pub search_error: Option<String>,
}

#[derive(InputObject)]
pub struct SearchReleasesInput {
    pub title_id: ID,
    pub series_movie_link_id: Option<ID>,
    pub season: Option<String>,
    pub episode: Option<String>,
    pub limit: Option<i32>,
}

#[derive(InputObject)]
pub struct QueueDownloadInput {
    pub title_id: ID,
    pub candidate_token: String,
    pub scope: QueueDownloadScopeInput,
    pub replace_in_progress: Option<bool>,
    pub purpose: Option<QueueDownloadPurposeValue>,
}

#[derive(InputObject)]
pub struct QueueBestReleaseInput {
    pub title_id: ID,
    pub scope: QueueDownloadScopeInput,
    pub replace_in_progress: Option<bool>,
}

/// Scope selector for the interactive acquisition-search job.
/// A bare input searches every derived target of `wanted_kind`; narrowing fields
/// filter that set. `wanted_item_id` (a state-row id or a convergence scope key)
/// resolves a single scope.
#[derive(InputObject)]
pub struct TriggerAcquisitionSearchInput {
    /// Which derived target set to search. Defaults to `Missing`.
    pub wanted_kind: Option<WantedKindValue>,
    /// Restrict to one facet.
    pub facet: Option<MediaFacetValue>,
    /// Restrict to these libraries.
    pub library_ids: Option<Vec<ID>>,
    /// Restrict to one title.
    pub title_id: Option<ID>,
    /// Restrict to one season of `title_id` (episodic facets).
    pub season_number: Option<i32>,
    /// Search exactly one scope: a state-row id or a convergence scope key
    /// (`episode:<uuid>`, `title:<uuid>`, `series_movie:<uuid>`, …).
    pub wanted_item_id: Option<ID>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum QueueDownloadResultStatusValue {
    Queued,
    Conflict,
}

#[derive(SimpleObject, Clone)]
pub struct QueueDownloadConflictPayload {
    pub title_id: ID,
    pub title_name: String,
    pub download_client_id: Option<ID>,
    pub download_client_type: String,
    pub download_client_item_id: String,
    pub source_title: Option<String>,
    pub source_kind: Option<DownloadSourceKindValue>,
    pub scope: QueueDownloadScopePayload,
    pub state: Option<DownloadQueueStateValue>,
    pub replaceable: bool,
}

#[derive(SimpleObject, Clone)]
pub struct QueueDownloadPayload {
    pub status: QueueDownloadResultStatusValue,
    pub job_id: Option<ID>,
    pub title_id: ID,
    pub title_name: String,
    pub source_title: Option<String>,
    pub source_kind: Option<DownloadSourceKindValue>,
    pub conflict: Option<QueueDownloadConflictPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct WantedSearchPayload {
    pub queued_count: i32,
    pub skipped_in_progress_count: i32,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum DownloadQueueActionKindValue {
    QueuedManualImport,
    IgnoredTrackedDownload,
    MarkedTrackedDownloadFailed,
    AssignedTrackedDownloadTitle,
    Paused,
    Resumed,
    DeleteQueued,
    Deleted,
}

#[derive(InputObject)]
pub struct QueueManualImportInput {
    pub selection_id: ID,
    pub files: Vec<ManualImportCandidateMappingInput>,
}

#[derive(InputObject)]
pub struct BeginManualImportSelectionInput {
    pub client_id: Option<ID>,
    pub client_type: String,
    pub download_client_item_id: String,
    pub title_id: ID,
}

#[derive(InputObject, Clone)]
pub struct MediaRenamePreviewInput {
    pub facet: MediaFacetValue,
    pub title_id: Option<ID>,
    pub dry_run: Option<bool>,
}

#[derive(InputObject, Clone)]
pub struct MediaRenameApplyInput {
    pub facet: MediaFacetValue,
    pub title_id: ID,
    pub fingerprint: String,
    pub idempotency_key: Option<String>,
}

#[derive(InputObject, Clone)]
pub struct MediaRenameBulkApplyInput {
    pub facet: MediaFacetValue,
    pub fingerprint: String,
    pub idempotency_key: Option<String>,
}

#[derive(InputObject, Clone)]
pub struct SubtitleLanguagePreferenceInput {
    pub code: String,
    pub hearing_impaired: Option<bool>,
    pub forced: Option<bool>,
}

#[derive(InputObject, Clone)]
pub struct DelayProfileInput {
    pub id: ID,
    pub name: String,
    pub usenet_delay_minutes: i32,
    pub torrent_delay_minutes: i32,
    pub preferred_protocol: DelayProfilePreferredProtocolValue,
    pub min_age_minutes: i32,
    pub bypass_score_threshold: Option<i32>,
    pub applies_to_facets: Vec<MediaFacetValue>,
    pub tags: Vec<String>,
    pub priority: i32,
    pub enabled: bool,
}

#[derive(InputObject, Clone)]
pub struct RootFolderInput {
    pub path: String,
    pub is_default: bool,
}

#[derive(InputObject, Clone)]
pub struct UpdateMediaSettingsInput {
    pub scope: ContentScopeValue,
    pub library_path: Option<String>,
    pub root_folders: Option<Vec<RootFolderInput>>,
    pub required_audio_languages: Option<Vec<String>>,
    pub folder_template: Option<String>,
    pub season_folder_template: Option<String>,
    pub specials_folder_template: Option<String>,
    pub rename_enabled: Option<bool>,
    pub rename_template: Option<String>,
    pub rename_collision_policy: Option<RenameCollisionPolicyValue>,
    pub rename_missing_metadata_policy: Option<RenameMissingMetadataPolicyValue>,
    pub filler_policy: Option<FillerPolicyValue>,
    pub recap_policy: Option<RecapPolicyValue>,
    pub monitor_specials: Option<bool>,
    pub inter_season_movies: Option<bool>,
    pub monitor_filler_movies: Option<bool>,
    pub nfo_write_on_import: Option<bool>,
    pub plexmatch_write_on_import: Option<bool>,
    pub import_mode: Option<ImportModeValue>,
    pub set_permissions_linux: Option<bool>,
    pub file_chmod: Option<String>,
    pub folder_chmod: Option<String>,
    pub chown_group: Option<String>,
}

#[derive(InputObject, Clone)]
pub struct UpdateLibraryPathsInput {
    pub movie_path: String,
    pub series_path: String,
    pub anime_path: Option<String>,
}

#[derive(InputObject, Clone)]
pub struct UpdateServiceSettingsInput {
    pub tls_cert_path: String,
    pub tls_key_path: String,
}

#[derive(InputObject, Clone)]
pub struct UpdateGeneralSettingsInput {
    pub keep_history_forever: Option<bool>,
    pub history_retention_days: Option<i32>,
    pub image_cache_max_size_mb: Option<i32>,
    pub plugin_http_ca_bundle_pem: Option<String>,
}

#[derive(InputObject, Clone)]
pub struct UpdateAutoBackupSettingsInput {
    pub enabled: bool,
    pub daily_time_local: String,
    pub set_auto_backup_key: Option<String>,
    pub clear_auto_backup_key: bool,
}

#[derive(InputObject, Clone)]
pub struct UpdateBackupSettingsInput {
    pub custom_backup_path: Option<String>,
}

#[derive(InputObject, Clone)]
pub struct UpdateSecuritySettingsInput {
    pub form_login_enabled: bool,
    pub password_min_length: i32,
    pub skip_login_for_local_ips: bool,
    pub mfa_require_config_step_up: bool,
    pub mfa_require_password_login: bool,
    pub totp_require_jellyfin_login: bool,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalAuthRuntimeConnectionPayload {
    pub id: ID,
    pub provider: ExternalAccountProviderValue,
    pub display_name: String,
    pub login_enabled: bool,
    pub linking_enabled: bool,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalAuthRuntimeSettingsPayload {
    pub login_providers: Vec<ExternalAccountProviderValue>,
    pub linking_providers: Vec<ExternalAccountProviderValue>,
    pub connections: Vec<ExternalAuthRuntimeConnectionPayload>,
}

#[derive(InputObject, Clone)]
pub struct CreateExternalAccountInviteInput {
    pub user_id: ID,
    pub connection_id: ID,
    pub provider: ExternalAccountProviderValue,
    pub provider_user_identifier: String,
    pub provider_user_id: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct MediaServerPathMappingPayload {
    pub source_path: String,
    pub destination_path: String,
}

#[derive(InputObject, Clone)]
pub struct MediaServerPathMappingInput {
    pub source_path: String,
    pub destination_path: String,
}

#[derive(SimpleObject, Clone)]
pub struct MediaServerDefaultLibraryGrantPayload {
    pub library_id: ID,
    pub permissions: Vec<LibraryPermissionValue>,
}

#[derive(InputObject, Clone)]
pub struct MediaServerDefaultLibraryGrantInput {
    pub library_id: ID,
    pub permissions: Vec<LibraryPermissionValue>,
}

#[derive(SimpleObject, Clone)]
pub struct MediaServerConnectionPayload {
    pub id: ID,
    pub provider: MediaServerProviderValue,
    pub display_name: String,
    pub base_url: String,
    pub enabled: bool,
    pub login_enabled: bool,
    pub linking_enabled: bool,
    pub auto_add_enabled: bool,
    pub default_app_permissions: Vec<AppPermissionValue>,
    pub default_library_grants: Vec<MediaServerDefaultLibraryGrantPayload>,
    pub machine_id_present: bool,
    pub api_key_present: bool,
    pub path_mappings: Vec<MediaServerPathMappingPayload>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct DeleteMediaServerConnectionPayload {
    pub id: async_graphql::ID,
}

#[derive(SimpleObject, Clone)]
pub struct JellyfinServerUserPayload {
    pub id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum MediaServerUserGroupStatusValue {
    Ready,
    MissingCredentials,
    Error,
}

#[derive(SimpleObject, Clone)]
pub struct MediaServerUserPayload {
    pub id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct MediaServerUserGroupPayload {
    pub connection_id: ID,
    pub connection_name: String,
    pub provider: ExternalAccountProviderValue,
    pub status: MediaServerUserGroupStatusValue,
    pub error_message: Option<String>,
    pub users: Vec<MediaServerUserPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct PlexServerDiscoveryPayload {
    pub id: String,
    pub name: String,
}

#[derive(InputObject, Clone)]
pub struct CreateMediaServerConnectionInput {
    pub provider: MediaServerProviderValue,
    pub display_name: String,
    pub base_url: String,
    pub enabled: Option<bool>,
    pub login_enabled: Option<bool>,
    pub linking_enabled: Option<bool>,
    pub auto_add_enabled: Option<bool>,
    pub default_app_permissions: Option<Vec<AppPermissionValue>>,
    pub default_library_grants: Option<Vec<MediaServerDefaultLibraryGrantInput>>,
    pub machine_id: Option<String>,
    pub plex_auth_token: Option<String>,
    pub plex_server_id: Option<String>,
    pub api_key: Option<String>,
    pub admin_username: Option<String>,
    pub admin_password: Option<String>,
    pub path_mappings: Option<Vec<MediaServerPathMappingInput>>,
}

#[derive(InputObject, Clone)]
pub struct UpdateMediaServerConnectionInput {
    pub id: ID,
    pub provider: Option<MediaServerProviderValue>,
    pub display_name: Option<String>,
    pub base_url: Option<String>,
    pub enabled: Option<bool>,
    pub login_enabled: Option<bool>,
    pub linking_enabled: Option<bool>,
    pub auto_add_enabled: Option<bool>,
    pub default_app_permissions: Option<Vec<AppPermissionValue>>,
    pub default_library_grants: Option<Vec<MediaServerDefaultLibraryGrantInput>>,
    pub machine_id: Option<String>,
    pub clear_machine_id: Option<bool>,
    pub plex_auth_token: Option<String>,
    pub plex_server_id: Option<String>,
    pub api_key: Option<String>,
    pub clear_api_key: Option<bool>,
    pub admin_username: Option<String>,
    pub admin_password: Option<String>,
    pub path_mappings: Option<Vec<MediaServerPathMappingInput>>,
}

#[derive(InputObject, Clone)]
pub struct TestMediaServerConnectionInput {
    pub id: ID,
    pub plex_auth_token: Option<String>,
}

#[derive(InputObject, Clone)]
pub struct LinkPlexAccountInput {
    pub connection_id: ID,
    pub plex_auth_token: String,
}

#[derive(InputObject, Clone)]
pub struct LinkJellyfinAccountInput {
    pub connection_id: ID,
    pub username: String,
    pub password: String,
}

#[derive(SimpleObject, Clone)]
pub struct UnlinkExternalAccountPayload {
    pub linked_account_id: ID,
}

#[derive(InputObject, Clone)]
pub struct UpdateSubtitleSettingsInput {
    pub enabled: bool,
    pub languages: Vec<SubtitleLanguagePreferenceInput>,
    pub auto_download_on_import: bool,
    pub minimum_score_series: i32,
    pub minimum_score_movie: i32,
    pub search_interval_hours: i32,
    pub include_ai_translated: bool,
    pub include_machine_translated: bool,
    pub sync_enabled: bool,
    pub sync_threshold_series: i32,
    pub sync_threshold_movie: i32,
    pub sync_max_offset_seconds: i32,
}

#[derive(InputObject, Clone)]
pub struct UpdateRecycleBinSettingsInput {
    pub enabled: bool,
}

#[derive(InputObject, Clone)]
pub struct UpdateAcquisitionSettingsInput {
    pub enabled: bool,
    pub upgrade_cooldown_hours: i32,
    pub same_tier_min_delta: i32,
    pub cross_tier_min_delta: i32,
    pub forced_upgrade_delta_bypass: i32,
    pub poll_interval_seconds: i32,
    pub long_tail_backfill_max_scopes_per_cycle: i32,
    pub long_tail_reconverge_days: i32,
}

#[derive(InputObject, Clone)]
pub struct ScoringOverridesInput {
    pub allow_x265_non4k: Option<bool>,
    pub block_dv_without_fallback: Option<bool>,
    pub prefer_compact_encodes: Option<bool>,
    pub prefer_lossless_audio: Option<bool>,
    pub block_upscaled: Option<bool>,
}

#[derive(InputObject, Clone)]
pub struct QualityProfileCriteriaInput {
    pub quality_tiers: Vec<String>,
    pub archival_quality: Option<String>,
    pub allow_unknown_quality: bool,
    pub source_allowlist: Vec<String>,
    pub source_blocklist: Vec<String>,
    pub video_codec_allowlist: Vec<String>,
    pub video_codec_blocklist: Vec<String>,
    pub audio_codec_allowlist: Vec<String>,
    pub audio_codec_blocklist: Vec<String>,
    pub dolby_vision_allowed: bool,
    pub detected_hdr_allowed: bool,
    pub prefer_remux: bool,
    pub allow_bd_disk: bool,
    pub allow_upgrades: bool,
    pub scoring_overrides: ScoringOverridesInput,
    pub cutoff_tier: Option<String>,
    pub min_score_to_grab: Option<i32>,
}

#[derive(InputObject, Clone)]
pub struct QualityProfileInput {
    pub id: ID,
    pub name: String,
    pub criteria: QualityProfileCriteriaInput,
}

#[derive(InputObject, Clone)]
pub struct QualityProfileSelectionInput {
    pub scope: ContentScopeValue,
    pub profile_id: Option<ID>,
    pub inherit_global: bool,
}

#[derive(InputObject, Clone)]
pub struct FacetScoringPersonaSelectionInput {
    pub scope: ContentScopeValue,
    pub persona: Option<ScoringPersonaValue>,
    pub inherit_global: bool,
}

#[derive(InputObject, Clone)]
pub struct SaveQualityProfileSettingsInput {
    pub profiles: Vec<QualityProfileInput>,
    pub global_profile_id: Option<ID>,
    pub global_scoring_persona: Option<ScoringPersonaValue>,
    pub category_selections: Vec<QualityProfileSelectionInput>,
    pub category_persona_selections: Vec<FacetScoringPersonaSelectionInput>,
    pub replace_existing: bool,
}

#[derive(InputObject, Clone)]
pub struct DownloadClientRoutingEntryInput {
    pub client_id: ID,
    pub enabled: bool,
    pub category: Option<String>,
    pub recent_queue_priority: Option<String>,
    pub older_queue_priority: Option<String>,
    pub remove_completed: bool,
    pub remove_failed: bool,
}

#[derive(InputObject, Clone)]
pub struct UpdateDownloadClientRoutingInput {
    pub scope: ContentScopeValue,
    pub entries: Vec<DownloadClientRoutingEntryInput>,
}

#[derive(InputObject, Clone)]
pub struct IndexerRoutingEntryInput {
    pub indexer_id: ID,
    pub enabled: bool,
    pub categories: Vec<String>,
    pub priority: i32,
}

#[derive(InputObject, Clone)]
pub struct UpdateIndexerRoutingInput {
    pub scope: ContentScopeValue,
    pub entries: Vec<IndexerRoutingEntryInput>,
}

#[derive(InputObject)]
pub struct CreateIndexerConfigInput {
    pub name: String,
    pub provider_type: String,
    pub indexer_proxy_config_id: Option<ID>,
    pub rate_limit_seconds: Option<i64>,
    pub rate_limit_burst: Option<i64>,
    pub is_enabled: Option<bool>,
    pub enable_interactive_search: Option<bool>,
    pub enable_auto_search: Option<bool>,
    pub config: Option<Vec<ProviderConfigValueInput>>,
}

#[derive(InputObject)]
pub struct UpdateIndexerConfigInput {
    pub id: ID,
    pub name: Option<String>,
    pub provider_type: Option<String>,
    pub indexer_proxy_config_id: MaybeUndefined<ID>,
    pub rate_limit_seconds: Option<i64>,
    pub rate_limit_burst: Option<i64>,
    pub is_enabled: Option<bool>,
    pub enable_interactive_search: Option<bool>,
    pub enable_auto_search: Option<bool>,
    pub config: Option<Vec<ProviderConfigValueInput>>,
}

#[derive(InputObject)]
pub struct CreateIndexerProxyConfigInput {
    pub name: String,
    pub provider_type: String,
    pub base_url: String,
    pub request_timeout_seconds: Option<i32>,
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
pub struct UpdateIndexerProxyConfigInput {
    pub id: ID,
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub request_timeout_seconds: Option<i32>,
    pub is_enabled: Option<bool>,
}

#[derive(SimpleObject, Clone)]
pub struct DeleteIndexerProxyConfigPayload {
    pub id: ID,
}

#[derive(SimpleObject, Clone)]
pub struct DeleteIndexerConfigPayload {
    pub id: async_graphql::ID,
}

#[derive(InputObject)]
pub struct CreateDownloadClientConfigInput {
    pub name: String,
    pub client_type: String,
    pub config: Vec<ProviderConfigValueInput>,
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
pub struct UpdateDownloadClientConfigInput {
    pub id: ID,
    pub name: Option<String>,
    pub client_type: Option<String>,
    pub config: Option<Vec<ProviderConfigValueInput>>,
    pub is_enabled: Option<bool>,
}

#[derive(SimpleObject, Clone)]
pub struct DeleteDownloadClientConfigPayload {
    pub id: async_graphql::ID,
}

#[derive(InputObject)]
pub struct ReorderDownloadClientConfigsInput {
    pub ids: Vec<ID>,
}

#[derive(SimpleObject, Clone)]
pub struct ReorderDownloadClientConfigsPayload {
    pub ids: Vec<ID>,
}

#[derive(InputObject)]
pub struct TestDownloadClientConnectionInput {
    pub id: Option<ID>,
    pub client_type: String,
    pub config: Vec<ProviderConfigValueInput>,
}

#[derive(InputObject)]
pub struct CreateSubtitleProviderConfigInput {
    pub name: String,
    pub provider_type: String,
    pub config: Vec<ProviderConfigValueInput>,
    pub enabled_facets: Option<Vec<MediaFacetValue>>,
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
pub struct UpdateSubtitleProviderConfigInput {
    pub id: ID,
    pub name: Option<String>,
    pub provider_type: Option<String>,
    pub config: Option<Vec<ProviderConfigValueInput>>,
    pub enabled_facets: Option<Vec<MediaFacetValue>>,
    pub is_enabled: Option<bool>,
    pub disabled_until: MaybeUndefined<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
pub struct DeleteSubtitleProviderConfigPayload {
    pub id: async_graphql::ID,
}

#[derive(InputObject)]
pub struct TestSubtitleProviderConnectionInput {
    pub id: Option<ID>,
    pub provider_type: String,
    pub config: Vec<ProviderConfigValueInput>,
}

#[derive(InputObject)]
pub struct TestIndexerConnectionInput {
    pub provider_type: String,
    pub config: Option<Vec<ProviderConfigValueInput>>,
    pub indexer_id: Option<ID>,
    pub indexer_proxy_config_id: MaybeUndefined<ID>,
}

#[derive(InputObject)]
pub struct DeleteTitleInput {
    pub title_id: ID,
    pub delete_files_on_disk: Option<bool>,
    pub preview_fingerprint: Option<String>,
    pub typed_confirmation: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct DeleteTitlePayload {
    pub id: async_graphql::ID,
}

#[derive(InputObject)]
pub struct DeleteTitlesInput {
    pub items: Vec<DeleteTitlesItemInput>,
    pub delete_files_on_disk: Option<bool>,
    pub typed_confirmation: Option<String>,
}

#[derive(InputObject)]
pub struct DeleteTitlesItemInput {
    pub title_id: ID,
    pub preview_fingerprint: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct ClearTitleReleaseBlocklistEntryPayload {
    pub id: async_graphql::ID,
}

#[derive(InputObject)]
pub struct DeleteTitlesPreviewInput {
    pub title_ids: Vec<ID>,
}

#[derive(InputObject)]
pub struct CreateUserInput {
    pub username: String,
    pub password: String,
    pub app_permissions: Vec<AppPermissionValue>,
    pub library_permissions: Vec<LibraryPermissionGrantInput>,
}

#[derive(InputObject)]
pub struct SetUserLoginEnabledInput {
    pub user_id: ID,
    pub enabled: bool,
}

#[derive(InputObject)]
pub struct SetUserPasswordInput {
    pub user_id: ID,
    pub password: String,
    pub current_password: Option<String>,
}

#[derive(InputObject)]
pub struct SetTitleMonitoredInput {
    pub title_id: ID,
    pub monitored: bool,
}

#[derive(InputObject)]
pub struct UpdateTitleInput {
    pub title_id: ID,
    pub name: Option<String>,
    pub facet: Option<MediaFacetValue>,
    pub tags: Option<Vec<String>>,
    pub options: Option<TitleOptionsInput>,
}

#[derive(InputObject)]
pub struct SetPrimaryMovieFileInput {
    pub title_id: ID,
    pub file_id: ID,
}

#[derive(InputObject)]
pub struct FixTitleMatchInput {
    pub title_id: ID,
    pub tvdb_id: String,
}

#[derive(InputObject, Clone)]
pub struct SetCollectionMonitoredInput {
    pub collection_id: ID,
    pub monitored: bool,
}

#[derive(InputObject, Clone)]
pub struct SetEpisodeMonitoredInput {
    pub episode_id: ID,
    pub monitored: bool,
}

#[derive(InputObject, Clone)]
pub struct SetSeriesMovieMonitoredInput {
    pub series_movie_link_id: ID,
    pub monitored: bool,
}

#[derive(InputObject)]
pub struct SetUserAppPermissionsInput {
    pub user_id: ID,
    pub permissions: Vec<AppPermissionValue>,
}

#[derive(SimpleObject, Clone)]
pub struct DeleteUserPayload {
    pub id: ID,
}

#[derive(InputObject, Clone)]
pub struct LibraryPermissionGrantInput {
    pub library_id: ID,
    pub permissions: Vec<LibraryPermissionValue>,
}

#[derive(InputObject, Clone)]
pub struct SetUserLibraryPermissionsInput {
    pub user_id: ID,
    pub grants: Vec<LibraryPermissionGrantInput>,
}

#[derive(InputObject, Clone)]
pub struct CreateLibraryRootInput {
    pub path: String,
    pub is_default: bool,
}

#[derive(InputObject, Clone)]
pub struct UpdateLibraryRootInput {
    pub path: String,
    pub is_default: bool,
}

#[derive(InputObject, Clone)]
pub struct CreateLibraryInput {
    pub facet: MediaFacetValue,
    pub name: String,
    pub roots: Vec<CreateLibraryRootInput>,
    pub settings: Option<LibrarySettingsInput>,
}

#[derive(InputObject, Clone)]
pub struct UpdateLibraryInput {
    pub library_id: ID,
    pub name: Option<String>,
    pub roots: Option<Vec<UpdateLibraryRootInput>>,
    pub settings: Option<LibrarySettingsInput>,
}

#[derive(InputObject, Clone)]
pub struct LibrarySettingsInput {
    pub required_audio_languages: Option<Vec<String>>,
    pub quality_profile_id: Option<ID>,
    pub request_quality_profile_ids: Option<Vec<ID>>,
    pub scoring_persona: Option<ScoringPersonaValue>,
    pub filler_policy: Option<FillerPolicyValue>,
    pub recap_policy: Option<RecapPolicyValue>,
    pub monitor_specials: Option<bool>,
    pub inter_season_movies: Option<bool>,
    pub monitor_filler_movies: Option<bool>,
    pub nfo_write_on_import: Option<bool>,
    pub plexmatch_write_on_import: Option<bool>,
    pub import_mode: Option<ImportModeValue>,
    pub set_permissions_linux: Option<bool>,
    pub file_chmod: Option<String>,
    pub folder_chmod: Option<String>,
    pub chown_group: Option<String>,
    pub indexer_routing: Option<Vec<IndexerRoutingEntryInput>>,
    pub download_client_routing: Option<Vec<DownloadClientRoutingEntryInput>>,
}

#[derive(SimpleObject, Clone)]
pub struct DeleteLibraryPayload {
    pub id: async_graphql::ID,
}

#[derive(InputObject)]
pub struct DeleteMediaFileInput {
    pub file_id: ID,
    pub delete_from_disk: Option<bool>,
    pub preview_fingerprint: Option<String>,
    pub typed_confirmation: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct DeleteMediaFilePayload {
    pub id: async_graphql::ID,
    pub job_run: JobRunPayload,
}

#[derive(InputObject)]
pub struct PauseDownloadInput {
    pub client_id: Option<ID>,
    pub download_client_item_id: String,
}

#[derive(InputObject)]
pub struct ResumeDownloadInput {
    pub client_id: Option<ID>,
    pub download_client_item_id: String,
}

#[derive(InputObject)]
pub struct DeleteDownloadInput {
    pub client_id: Option<ID>,
    pub client_type: String,
    pub download_client_item_id: String,
    pub is_history: bool,
}

// --- Manual Import ---

#[derive(SimpleObject, Clone)]
pub struct ManualImportFilePreviewPayload {
    pub candidate_id: ID,
    pub file_name: String,
    pub size_bytes: Long,
    pub quality: Option<String>,
    pub parsed_season: Option<i32>,
    pub parsed_episodes: Vec<i32>,
    pub suggested_episode_id: Option<ID>,
    pub suggested_episode_label: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct ManualImportSeriesMovieTargetPayload {
    pub series_movie_link_id: String,
    pub movie_title: String,
    pub year: Option<i32>,
    pub runtime_minutes: Option<i32>,
}

#[derive(InputObject)]
pub struct ManualImportCandidateMappingInput {
    pub candidate_id: ID,
    pub episode_id: Option<ID>,
    pub series_movie_link_id: Option<ID>,
}

// --- Wanted Items / Acquisition ---

#[derive(SimpleObject, Clone)]
pub struct DecisionCodeCountPayload {
    pub code: String,
    pub count: i64,
}

#[derive(SimpleObject, Clone)]
pub struct WantedStatusCountPayload {
    pub status: WantedStatusValue,
    pub count: i64,
}

#[derive(SimpleObject, Clone)]
pub struct PendingReleaseStatusCountPayload {
    pub status: PendingReleaseStatusValue,
    pub count: i64,
}

#[derive(SimpleObject, Clone)]
pub struct CutoffUnmetItemPayload {
    pub title_id: ID,
    pub title_name: String,
    pub title_slug: Option<String>,
    pub title_facet: MediaFacetValue,
    pub library_id: ID,
    pub library_name: Option<String>,
    pub library_slug: Option<String>,
    pub episode_id: Option<ID>,
    pub season_number: Option<String>,
    pub episode_number: Option<String>,
    pub current_tier: String,
    pub target_tier: String,
    /// Convergence progress for this upgrade scope — the same
    /// state the Missing/Upgrades views show.
    pub convergence_state: ConvergenceStateValue,
    pub indexers_covered: i32,
    pub indexers_routed: i32,
}

/// Bounded view: one page of cutoff-unmet targets + the full unmet
/// count, so the UI paginates instead of loading the whole set.
#[derive(SimpleObject, Clone)]
pub struct CutoffUnmetTitlesPagePayload {
    pub items: Vec<CutoffUnmetItemPayload>,
    pub total_count: i64,
    pub has_more: bool,
}

#[derive(SimpleObject, Clone)]
pub struct PauseWantedItemPayload {
    pub id: async_graphql::ID,
}

#[derive(SimpleObject, Clone)]
pub struct ResumeWantedItemPayload {
    pub id: async_graphql::ID,
}

#[derive(SimpleObject, Clone)]
pub struct TriggerTitleMismatchRecoverySearchPayload {
    pub title_id: ID,
    pub queued_count: i32,
}

/// Lifecycle of the interactive acquisition-search job.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum AcquisitionSearchJobStateValue {
    Running,
    Completed,
    Cancelled,
    Failed,
}

/// Progress snapshot for the interactive acquisition-search job.
/// Survives navigation/refresh — the job runs server-side and its state is queried
/// by `id` and pushed via `jobRunEvents`.
#[derive(SimpleObject, Clone)]
pub struct AcquisitionSearchJobPayload {
    pub id: ID,
    pub state: AcquisitionSearchJobStateValue,
    pub total: i32,
    pub processed: i32,
    pub grabbed_count: i32,
    pub failed_count: i32,
    pub current_title: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// Lifecycle of an interactive release-search job. The job runs server-side;
/// results stream into the snapshot as each indexer completes.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum InteractiveReleaseSearchStateValue {
    Running,
    Completed,
    Cancelled,
}

/// Per-indexer progress within an interactive release-search job.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum InteractiveReleaseSearchIndexerStatusValue {
    Pending,
    Searching,
    Completed,
    Failed,
    Skipped,
}

/// Status of a single indexer inside an interactive release-search job.
#[derive(SimpleObject, Clone)]
pub struct InteractiveReleaseSearchIndexerPayload {
    pub indexer_id: ID,
    pub name: String,
    pub status: InteractiveReleaseSearchIndexerStatusValue,
    /// The indexer's own result count (before cross-indexer dedup).
    pub result_count: i32,
    pub failure_reason: Option<String>,
}

/// Snapshot of an interactive release-search job, polled by `id` while the
/// job runs so results appear as each indexer completes.
#[derive(SimpleObject, Clone)]
pub struct InteractiveReleaseSearchPayload {
    pub id: ID,
    pub state: InteractiveReleaseSearchStateValue,
    /// Scored, cross-indexer-deduped snapshot of the merged results so far.
    pub results: Vec<IndexerSearchResultPayload>,
    pub indexers: Vec<InteractiveReleaseSearchIndexerPayload>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
pub struct CancelInteractiveReleaseSearchPayload {
    pub id: ID,
    /// False when the search had already finished — not an error.
    pub accepted: bool,
}

// ── Rule Sets ──────────────────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
pub struct RuleSetPayload {
    pub id: ID,
    pub name: String,
    pub description: String,
    pub rego_source: String,
    pub enabled: bool,
    pub priority: i32,
    pub applied_facets: Vec<String>,
    pub is_managed: bool,
    pub managed_key: Option<String>,
    /// Tags a managed pack is narrowed to. Null means it applies wherever its
    /// facts match. Always null for user-authored rule sets.
    pub managed_tag_filter: Option<Vec<String>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct DeleteRuleSetPayload {
    pub id: async_graphql::ID,
}

#[derive(SimpleObject, Clone)]
pub struct RuleValidationResultPayload {
    pub valid: bool,
    pub errors: Vec<String>,
}

#[derive(InputObject)]
pub struct CreateRuleSetInput {
    pub name: String,
    pub description: Option<String>,
    pub rego_source: String,
    pub applied_facets: Option<Vec<String>>,
    pub priority: Option<i32>,
    pub enabled: Option<bool>,
}

#[derive(InputObject)]
pub struct UpdateRuleSetInput {
    pub id: ID,
    pub name: Option<String>,
    pub description: Option<String>,
    pub rego_source: Option<String>,
    pub applied_facets: Option<Vec<String>>,
    pub priority: Option<i32>,
    /// Narrow a managed locale pack to titles carrying one of these tags. An
    /// empty list clears the filter so the pack applies wherever its facts
    /// match. Rejected for user-authored rule sets.
    pub managed_tag_filter: Option<Vec<String>>,
}

#[derive(InputObject)]
pub struct ToggleRuleSetInput {
    pub id: ID,
    pub enabled: bool,
}

#[derive(InputObject)]
pub struct ValidateRuleSetInput {
    pub rego_source: String,
    pub rule_set_id: Option<ID>,
}

#[derive(InputObject)]
pub struct SetTitleRequiredAudioInput {
    pub title_id: ID,
    /// The facet of the title: "movie", "series", or "anime"
    pub facet: MediaFacetValue,
    /// `null` removes the override and inherits from the facet.
    /// `[]` stores an explicit "no required languages" override for the title.
    pub languages: Option<Vec<String>>,
}

#[derive(SimpleObject, Clone)]
pub struct SetTitleRequiredAudioPayload {
    pub title_id: ID,
    pub facet: MediaFacetValue,
    pub languages: Option<Vec<String>>,
    pub updated: bool,
}

#[derive(SimpleObject, Clone)]
pub struct ServiceLogsPayload {
    pub generated_at: DateTime<Utc>,
    pub lines: Vec<String>,
    pub count: i32,
}

// ── Metadata Gateway (proxied from SMG) ────────────────────────────────────

#[derive(InputObject, Clone)]
pub struct MetadataMovieInput {
    pub tvdb_id: String,
    pub language: Option<String>,
}

#[derive(InputObject, Clone)]
pub struct MetadataSeriesInput {
    pub tvdb_id: String,
    pub include_episodes: Option<bool>,
    pub language: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct MetadataSearchItemPayload {
    pub tvdb_id: String,
    pub name: String,
    pub imdb_id: Option<String>,
    pub slug: Option<String>,
    #[graphql(name = "type")]
    pub type_hint: Option<String>,
    pub year: Option<i32>,
    pub status: Option<String>,
    pub overview: Option<String>,
    pub popularity: Option<f64>,
    pub poster_url: Option<String>,
    pub language: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub sort_title: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct MetadataSearchMultiPayload {
    pub movies: Vec<MetadataSearchItemPayload>,
    pub series: Vec<MetadataSearchItemPayload>,
    pub anime: Vec<MetadataSearchItemPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct MetadataMoviePayload {
    pub tvdb_id: String,
    pub name: String,
    pub slug: String,
    pub year: Option<i32>,
    pub status: String,
    pub overview: String,
    pub poster_url: String,
    pub language: String,
    pub runtime_minutes: i32,
    pub sort_title: String,
    pub imdb_id: String,
    pub studio: String,
    pub tmdb_release_date: Option<Date>,
}

#[derive(SimpleObject, Clone)]
pub struct MetadataSeriesPayload {
    pub tvdb_id: String,
    pub name: String,
    pub sort_name: String,
    pub slug: String,
    pub year: Option<i32>,
    pub status: String,
    pub first_aired: Date,
    pub overview: String,
    pub network: String,
    pub runtime_minutes: i32,
    pub poster_url: String,
    pub country: String,
    pub aliases: Vec<String>,
    pub seasons: Vec<MetadataSeasonPayload>,
    pub episodes: Vec<MetadataEpisodePayload>,
}

#[derive(SimpleObject, Clone)]
pub struct MetadataSeasonPayload {
    pub tvdb_id: String,
    pub number: i32,
    pub label: String,
    pub episode_type: String,
}

#[derive(SimpleObject, Clone)]
pub struct MetadataEpisodePayload {
    pub tvdb_id: String,
    pub episode_number: i32,
    pub season_number: i32,
    pub name: String,
    pub aired: Date,
    pub runtime_minutes: i32,
    pub is_filler: bool,
    pub image_url: String,
}

#[derive(SimpleObject, Clone)]
pub struct CalendarEpisodePayload {
    pub id: ID,
    pub title_id: ID,
    pub library_id: ID,
    pub library_name: Option<String>,
    pub library_slug: Option<String>,
    pub title_name: String,
    pub title_slug: Option<String>,
    pub title_facet: String,
    pub season_number: Option<String>,
    pub episode_number: Option<String>,
    pub episode_title: Option<String>,
    pub air_date: Option<Date>,
    pub monitored: bool,
}

// ── Plugins ────────────────────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
pub struct RegistryPluginPayload {
    pub id: ID,
    pub name: String,
    pub description: String,
    pub version: String,
    pub latest_version: Option<String>,
    pub plugin_type: String,
    pub provider_type: String,
    pub author: String,
    pub official: bool,
    pub publisher: Option<String>,
    pub support_tier: String,
    pub status: Option<String>,
    pub docs_url: Option<String>,
    pub source_repo: Option<String>,
    pub builtin: bool,
    pub source_url: Option<String>,
    pub source_kind: Option<String>,
    pub blocked_reason: Option<String>,
    pub bytes: Option<Long>,
    pub is_installed: bool,
    pub is_enabled: bool,
    pub installed_version: Option<String>,
    pub update_available: bool,
    pub install_in_progress: bool,
    pub default_base_url: Option<String>,
}

// ── Rule Packs ────────────────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
pub struct RulePackRegistryEntryPayload {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub version: String,
}

#[derive(SimpleObject, Clone)]
pub struct RulePackTemplatePayload {
    pub id: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub rego_source: String,
    pub applied_facets: Vec<String>,
}

#[derive(SimpleObject, Clone)]
pub struct PluginInstallationPayload {
    pub id: ID,
    pub plugin_id: ID,
    pub name: String,
    pub description: String,
    pub version: String,
    pub sdk_version: String,
    pub sdk_constraint: String,
    pub plugin_type: String,
    pub provider_type: String,
    pub is_enabled: bool,
    pub is_builtin: bool,
    pub source_kind: String,
    pub source_url: Option<String>,
    pub publisher: Option<String>,
    pub support_tier: String,
    pub docs_url: Option<String>,
    pub source_repo: Option<String>,
    pub manifest_url: Option<String>,
    pub wasm_digest: Option<String>,
    pub artifact_digest: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject)]
pub struct PluginCatalogStatusPayload {
    pub refresh_state: CatalogRefreshStateValue,
    pub github_available: bool,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub outage_message: Option<String>,
    pub blocked_actions: Vec<String>,
    pub restore_warnings: Vec<String>,
    pub last_error: Option<String>,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum PluginInstallOperationKindValue {
    Install,
    Upgrade,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum PluginInstallStateValue {
    Downloading,
    Verifying,
    Installing,
    Succeeded,
    Failed,
}

#[derive(SimpleObject, Clone)]
pub struct PluginInstallProgressPayload {
    pub plugin_id: ID,
    pub operation_kind: PluginInstallOperationKindValue,
    pub state: PluginInstallStateValue,
    pub label: String,
    pub step_index: i32,
    pub step_count: i32,
    pub message: Option<String>,
    pub error: Option<String>,
}

#[derive(SimpleObject)]
pub struct ManualPluginPreviewPayload {
    pub github_repo_url: String,
    pub plugin: RegistryPluginPayload,
}

#[derive(InputObject)]
pub struct ManualPluginRepoInput {
    pub github_repo_url: String,
}

#[derive(InputObject)]
pub struct ManualPluginUploadInput {
    pub file_name: String,
    pub wasm_base64: String,
    pub acknowledge_risk: bool,
}

#[derive(SimpleObject, Clone)]
pub struct UninstallPluginPayload {
    pub plugin_id: async_graphql::ID,
}

#[derive(InputObject)]
pub struct TogglePluginInput {
    pub plugin_id: ID,
    pub enabled: bool,
}

// ── Provider Type Config Schema ─────────────────────────────────────────

#[derive(SimpleObject, Clone)]
pub struct PluginConfigFieldOptionPayload {
    pub value: String,
    pub label: String,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum PluginConfigFieldTypeValue {
    String,
    Password,
    Multiline,
    Bool,
    Select,
    Number,
    Path,
    Tag,
}

impl PluginConfigFieldTypeValue {
    pub fn from_domain(value: ConfigFieldType) -> Self {
        match value {
            ConfigFieldType::String => Self::String,
            ConfigFieldType::Password => Self::Password,
            ConfigFieldType::Multiline => Self::Multiline,
            ConfigFieldType::Bool => Self::Bool,
            ConfigFieldType::Select => Self::Select,
            ConfigFieldType::Number => Self::Number,
            ConfigFieldType::Path => Self::Path,
            ConfigFieldType::Tag => Self::Tag,
        }
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum PluginConfigValueSourceValue {
    User,
    HostBinding,
}

impl PluginConfigValueSourceValue {
    pub fn from_domain(value: ConfigFieldValueSource) -> Self {
        match value {
            ConfigFieldValueSource::User => Self::User,
            ConfigFieldValueSource::HostBinding => Self::HostBinding,
        }
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum PluginConfigFieldRoleValue {
    ConnectionUrl,
}

impl PluginConfigFieldRoleValue {
    pub fn from_domain(value: ConfigFieldRole) -> Self {
        match value {
            ConfigFieldRole::ConnectionUrl => Self::ConnectionUrl,
        }
    }
}

#[derive(SimpleObject, Clone)]
pub struct PluginConfigFieldPayload {
    pub key: String,
    pub label: String,
    pub field_type: PluginConfigFieldTypeValue,
    pub required: bool,
    pub default_value: Option<String>,
    pub value_source: PluginConfigValueSourceValue,
    pub role: Option<PluginConfigFieldRoleValue>,
    pub host_binding: Option<String>,
    pub options: Vec<PluginConfigFieldOptionPayload>,
    pub help_text: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct ProviderTypePayload {
    pub provider_type: String,
    pub name: String,
    pub config_fields: Vec<PluginConfigFieldPayload>,
    pub default_base_url: Option<String>,
    pub available_host_bindings: Vec<String>,
    pub recommended_facets: Vec<MediaFacetValue>,
    pub supported_events: Vec<String>,
    pub supports_test: bool,
}

#[derive(SimpleObject, Clone)]
pub struct ProviderValidationPayload {
    pub status: String,
    pub message: Option<String>,
    pub retry_after_seconds: Option<i64>,
}

// ── Notification types ─────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
pub struct NotificationChannelPayload {
    pub id: ID,
    pub name: String,
    pub channel_type: String,
    pub config: Vec<ProviderConfigValuePayload>,
    pub stored_secret_keys: Vec<String>,
    pub media_server_connection_id: Option<ID>,
    pub is_enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct NotificationSubscriptionPayload {
    pub id: ID,
    pub channel_id: Option<ID>,
    pub target_kind: String,
    pub target_id: ID,
    pub event_type: String,
    pub scope: String,
    pub scope_id: Option<String>,
    pub is_enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct DeleteNotificationChannelPayload {
    pub id: async_graphql::ID,
}

#[derive(SimpleObject, Clone)]
pub struct NotificationChannelTestPayload {
    pub id: async_graphql::ID,
    pub status: String,
    pub message: Option<String>,
    pub retry_after_seconds: Option<i64>,
}

#[derive(SimpleObject, Clone)]
pub struct DeleteNotificationSubscriptionPayload {
    pub id: async_graphql::ID,
}

#[derive(SimpleObject, Clone)]
pub struct NotificationTargetPayload {
    pub id: ID,
    pub target_kind: String,
    pub name: String,
    pub provider_type: String,
    pub media_server_provider: Option<MediaServerProviderValue>,
    pub media_server_connection_id: Option<ID>,
    pub is_enabled: bool,
}

#[derive(InputObject)]
pub struct CreateNotificationChannelInput {
    pub name: String,
    pub channel_type: String,
    pub config: Vec<ProviderConfigValueInput>,
    pub media_server_connection_id: Option<ID>,
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
pub struct UpdateNotificationChannelInput {
    pub id: ID,
    pub name: Option<String>,
    pub config: Option<Vec<ProviderConfigValueInput>>,
    pub media_server_connection_id: Option<Option<ID>>,
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
pub struct CreateNotificationSubscriptionInput {
    pub channel_id: Option<ID>,
    pub target_kind: Option<String>,
    pub target_id: Option<ID>,
    pub event_type: String,
    pub scope: String,
    pub scope_id: Option<String>,
    pub is_enabled: Option<bool>,
}

#[derive(InputObject)]
pub struct UpdateNotificationSubscriptionInput {
    pub id: ID,
    pub target_kind: Option<String>,
    pub target_id: Option<ID>,
    pub event_type: Option<String>,
    pub scope: Option<String>,
    pub scope_id: Option<String>,
    pub is_enabled: Option<bool>,
}

/// Notification provider type payload (reuses the same shape as indexer provider types)
#[derive(SimpleObject, Clone)]
pub struct NotificationProviderTypePayload {
    pub provider_type: String,
    pub name: String,
    pub config_fields: Vec<PluginConfigFieldPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct BackupRowCountPayload {
    pub table: String,
    pub row_count: Long,
}

#[derive(SimpleObject, Clone)]
pub struct BackupInfoPayload {
    pub filename: String,
    pub size_bytes: Long,
    pub created_at: DateTime<Utc>,
    pub format_version: String,
    pub source_engine: String,
    pub source_migration_key: Option<String>,
    pub encrypted: bool,
    pub row_counts: Vec<BackupRowCountPayload>,
    pub trigger: String,
    pub status: String,
    pub error_message: Option<String>,
}

#[derive(InputObject)]
pub struct CreateBackupInput {
    pub password: String,
}

#[derive(InputObject)]
pub struct PrepareBackupDownloadInput {
    pub filename: String,
}

#[derive(InputObject)]
pub struct DeleteBackupInput {
    pub filename: String,
}

#[derive(SimpleObject, Clone)]
pub struct DeleteBackupPayload {
    pub filename: String,
    /// False when no backup file with that name existed — not an error.
    pub deleted: bool,
}

#[derive(SimpleObject, Clone)]
pub struct RssSyncReportPayload {
    pub releases_fetched: i32,
    pub releases_matched: i32,
    pub releases_grabbed: i32,
    pub releases_held: i32,
}

#[derive(SimpleObject, Clone)]
pub struct ForceGrabPendingReleasePayload {
    pub id: async_graphql::ID,
    /// False when the grab was rejected (e.g. the release is blocklisted)
    /// rather than queued — not an error.
    pub grabbed: bool,
}

#[derive(SimpleObject, Clone)]
pub struct DismissPendingReleasePayload {
    pub id: async_graphql::ID,
}

// ── Recycle Bin ────────────────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
pub struct RecycledItemPayload {
    pub id: async_graphql::ID,
    pub original_path: String,
    pub file_name: String,
    pub size_bytes: Long,
    pub title_id: Option<async_graphql::ID>,
    pub reason: String,
    pub recycled_at: DateTime<Utc>,
    pub media_root: String,
    pub library_id: async_graphql::ID,
    pub library_name: String,
}

#[derive(SimpleObject, Clone)]
pub struct RecycledItemsPayload {
    pub items: Vec<RecycledItemPayload>,
    pub total_count: i32,
}

#[derive(SimpleObject, Clone)]
pub struct RestoreRecycledItemPayload {
    pub id: async_graphql::ID,
    pub job_run: JobRunPayload,
}

#[derive(SimpleObject, Clone)]
pub struct DeleteRecycledItemPayload {
    pub id: async_graphql::ID,
    /// False when the entry was quarantined instead of purged (unsafe to
    /// delete) — the file remains on disk.
    pub deleted: bool,
}

#[derive(SimpleObject, Clone)]
pub struct EmptyRecycleBinPayload {
    pub purged_count: i32,
}

#[derive(SimpleObject, Clone)]
pub struct CompleteSetupPayload {
    pub completed: bool,
}

#[derive(SimpleObject, Clone)]
pub struct ClearTitleImageCachePayload {
    /// When the cache-clear request was accepted.
    pub requested_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct SetupStatusPayload {
    pub setup_complete: bool,
    pub has_download_clients: bool,
    pub has_indexers: bool,
}

#[derive(SimpleObject, Clone)]
pub struct DirectoryEntryPayload {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
}

// ── External Import (Sonarr/Radarr) ────────────────────────────────────────

#[derive(InputObject, Clone)]
pub struct ExternalImportConnectionInput {
    pub base_url: String,
    pub api_key: String,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ExternalArrSourceKind {
    Sonarr,
    Radarr,
}

/// Connection kind for the lightweight, pre-warmup connection probe used by the
/// setup wizard's Connect step. Unlike [`ExternalArrSourceKind`] this also
/// covers Prowlarr, whose child discovery starts separately after this probe.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ExternalImportConnectionKind {
    Sonarr,
    Radarr,
    Prowlarr,
}

#[derive(InputObject)]
pub struct ValidateExternalImportConnectionInput {
    pub kind: ExternalImportConnectionKind,
    pub connection: ExternalImportConnectionInput,
}

#[derive(InputObject)]
pub struct ExternalImportSetupInstanceApiKeyInput {
    pub instance_id: ID,
    pub kind: ExternalImportConnectionKind,
    pub api_key: String,
}

#[derive(InputObject)]
pub struct SaveExternalImportSetupSecretDraftInput {
    pub instance_api_keys: Vec<ExternalImportSetupInstanceApiKeyInput>,
    pub download_client_api_key_overrides: Vec<DownloadClientApiKeyOverrideInput>,
    pub download_client_password_overrides: Vec<DownloadClientPasswordOverrideInput>,
    pub indexer_api_key_overrides: Vec<IndexerApiKeyOverrideInput>,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportSetupInstanceApiKeyPayload {
    pub instance_id: ID,
    pub kind: ExternalImportConnectionKind,
    pub api_key: String,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportSetupApiKeyOverridePayload {
    pub dedup_key: String,
    pub api_key: String,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportSetupPasswordOverridePayload {
    pub dedup_key: String,
    pub password: String,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportSetupSecretDraftPayload {
    pub instance_api_keys: Vec<ExternalImportSetupInstanceApiKeyPayload>,
    pub download_client_api_key_overrides: Vec<ExternalImportSetupApiKeyOverridePayload>,
    pub download_client_password_overrides: Vec<ExternalImportSetupPasswordOverridePayload>,
    pub indexer_api_key_overrides: Vec<ExternalImportSetupApiKeyOverridePayload>,
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportSetupSecretDraftStatusPayload {
    pub has_draft: bool,
    pub owned_by_current_user: bool,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(SimpleObject, Clone)]
pub struct SaveExternalImportSetupSecretDraftPayload {
    pub overwrote_another_user_draft: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct ClearExternalImportSetupSecretDraftPayload {
    /// False when there was no draft owned by the caller to clear.
    pub cleared: bool,
}

#[derive(InputObject)]
pub struct StartExternalImportArrSourceWarmupInput {
    pub kind: ExternalArrSourceKind,
    pub connection: ExternalImportConnectionInput,
}

#[derive(InputObject)]
pub struct StartExternalImportProwlarrWarmupInput {
    pub connection: ExternalImportConnectionInput,
}

#[derive(InputObject)]
pub struct ExternalImportAggregateWarmupProgressInput {
    pub source_warmup_session_ids: Vec<ID>,
}

#[derive(InputObject)]
pub struct PreviewExternalImportInput {
    pub source_warmup_session_ids: Vec<ID>,
    pub prowlarr_warmup_session_id: Option<ID>,
    #[graphql(deprecation = "use prowlarrWarmupSessionId")]
    pub prowlarr: Option<ExternalImportConnectionInput>,
}

/// API key supplied by the user for a download client whose key was masked by
/// Sonarr/Radarr and could not be retrieved automatically.
#[derive(InputObject)]
pub struct DownloadClientApiKeyOverrideInput {
    pub dedup_key: String,
    pub api_key: String,
}

/// Password supplied by the user for a download client whose password was
/// masked by Sonarr/Radarr and could not be retrieved automatically.
#[derive(InputObject)]
pub struct DownloadClientPasswordOverrideInput {
    pub dedup_key: String,
    pub password: String,
}

/// API key supplied by the user for a grouped Prowlarr import candidate whose
/// key was masked or conflicted in Sonarr/Radarr, or for another indexer
/// whose key could not be retrieved automatically.
#[derive(InputObject)]
pub struct IndexerApiKeyOverrideInput {
    pub dedup_key: String,
    pub api_key: String,
}

#[derive(InputObject)]
pub struct ExecuteExternalImportInput {
    pub source_warmup_session_ids: Vec<ID>,
    pub prowlarr: Option<ExternalImportConnectionInput>,
    pub selected_download_client_dedup_keys: Vec<String>,
    pub selected_indexer_dedup_keys: Vec<String>,
    /// User-supplied API keys for download clients whose keys were masked by
    /// Sonarr/Radarr.  Keyed by the client's `dedup_key`.
    pub download_client_api_key_overrides: Vec<DownloadClientApiKeyOverrideInput>,
    /// User-supplied passwords for download clients whose passwords were
    /// masked by Sonarr/Radarr.  Keyed by the client's `dedup_key`.
    pub download_client_password_overrides: Vec<DownloadClientPasswordOverrideInput>,
    /// User-supplied API keys for grouped indexer import candidates, keyed by
    /// the import candidate's `dedup_key`.
    pub indexer_api_key_overrides: Vec<IndexerApiKeyOverrideInput>,
}

#[derive(InputObject)]
pub struct ExternalImportSourceLibraryMappingInput {
    /// Warmup session that surfaced this root. `None` for a manually-added root
    /// that no Sonarr/Radarr instance reported: such a root carries no
    /// monitored-status snapshot and simply registers its path on the target
    /// library. When set, `source_key` and `kind` must also be set.
    pub source_warmup_session_id: Option<ID>,
    pub source_key: Option<String>,
    pub kind: Option<ExternalArrSourceKind>,
    pub arr_root_path: String,
    pub scryer_root_path: String,
    pub library_id: ID,
    pub facet: MediaFacetValue,
}

#[derive(InputObject)]
pub struct FinalizeExternalImportInput {
    pub source_warmup_session_ids: Vec<ID>,
    pub mappings: Vec<ExternalImportSourceLibraryMappingInput>,
}

#[derive(SimpleObject, Clone)]
pub struct CancelExternalImportMonitorWarmupPayload {
    pub session_id: ID,
    /// False when the warmup had already finished — not an error.
    pub canceled: bool,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportConnectionValidationPayload {
    pub kind: ExternalImportConnectionKind,
    pub base_url: String,
    pub connected: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportAggregateWarmupProgressPayload {
    pub status: ExternalImportMonitorWarmupStatusValue,
    pub titles_total_known: bool,
    pub titles_fetched: i32,
    pub titles_total: i32,
    pub error_message: Option<String>,
}

#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ExternalImportLibrarySettingKey {
    RenameEnabled,
    NfoWriteOnImport,
    PlexmatchWriteOnImport,
    SetPermissionsLinux,
    FolderChmod,
    ChownGroup,
    QualityProfileId,
    RequestQualityProfileIds,
    MonitorSpecials,
    RenameTemplate,
    FolderTemplate,
    RequiredAudioLanguages,
}

#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ExternalImportLibrarySettingConfidence {
    High,
    Medium,
    Low,
}

#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ExternalImportLibrarySettingDisposition {
    AutoApplied,
    Suggested,
    Skipped,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportLibrarySettingValuePayload {
    pub bool_value: Option<bool>,
    pub string_value: Option<String>,
    pub string_list_value: Option<Vec<String>>,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportLibrarySettingEvidencePayload {
    pub source_key: String,
    pub source_kind: ExternalArrSourceKind,
    pub matching_count: i32,
    pub total_count: i32,
    pub detail: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportLibrarySettingApplicationPayload {
    pub library_id: ID,
    pub facet: MediaFacetValue,
    pub setting: ExternalImportLibrarySettingKey,
    pub value: ExternalImportLibrarySettingValuePayload,
    pub confidence: ExternalImportLibrarySettingConfidence,
    pub disposition: ExternalImportLibrarySettingDisposition,
    pub evidence: Vec<ExternalImportLibrarySettingEvidencePayload>,
    pub reason: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct FinalizeExternalImportPayload {
    pub monitor_warmup_session_id: ID,
}

#[derive(InputObject)]
pub struct RehydrateAllMetadataInput {
    pub language: String,
}

#[derive(SimpleObject, Clone)]
pub struct RehydrateAllMetadataPayload {
    pub language: String,
    pub titles_cleared: i64,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ExternalImportMonitorWarmupStatusValue {
    Queued,
    Running,
    Completed,
    Canceled,
    Failed,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum ExternalImportMonitorWarmupPhaseValue {
    LoadingIndexers,
    LoadingMovies,
    LoadingSeries,
    LoadingEpisodes,
    BuildingSnapshot,
    Ready,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportMonitorWarmupProgressPayload {
    pub session_id: ID,
    pub status: ExternalImportMonitorWarmupStatusValue,
    pub phase: ExternalImportMonitorWarmupPhaseValue,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub overall_total_known: bool,
    pub overall_progress: LibraryScanPhaseProgressPayload,
    pub movies_total_known: bool,
    pub movies_progress: LibraryScanPhaseProgressPayload,
    pub series_total_known: bool,
    pub series_progress: LibraryScanPhaseProgressPayload,
    pub episode_fetch_total_known: bool,
    pub episode_fetch_expected_total: Option<i32>,
    pub episode_fetch_expected_monitored_total: Option<i32>,
    pub episode_fetch_progress: LibraryScanPhaseProgressPayload,
    pub snapshot_build_total_known: bool,
    pub snapshot_build_progress: LibraryScanPhaseProgressPayload,
    pub matched_movie_count: i32,
    pub matched_series_count: i32,
    pub unmatched_movie_count: i32,
    pub unmatched_series_count: i32,
    pub ambiguous_movie_count: i32,
    pub ambiguous_series_count: i32,
    pub error_message: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportPreviewPayload {
    pub prowlarr_connected: bool,
    pub prowlarr_version: Option<String>,
    pub prowlarr_error: Option<String>,
    pub arr_sources: Vec<ExternalImportArrSourcePayload>,
    pub root_folders: Vec<ExternalImportRootFolderPayload>,
    pub download_clients: Vec<ExternalImportDownloadClientPayload>,
    pub indexers: Vec<ExternalImportIndexerPayload>,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportArrSourcePayload {
    pub session_id: ID,
    pub source_key: String,
    pub kind: ExternalArrSourceKind,
    pub base_url: String,
    pub connected: bool,
    pub version: Option<String>,
    pub status: ExternalImportMonitorWarmupStatusValue,
    pub error: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportRootFolderPayload {
    pub source_warmup_session_id: ID,
    pub source_key: String,
    pub kind: ExternalArrSourceKind,
    pub arr_root_path: String,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportDownloadClientPayload {
    pub source_keys: Vec<String>,
    pub name: String,
    pub implementation: String,
    pub scryer_client_type: Option<String>,
    pub host: Option<String>,
    /// Port as reported by the source Sonarr/Radarr instance (external
    /// passthrough; not guaranteed numeric).
    pub port: Option<String>,
    pub use_ssl: bool,
    pub url_base: Option<String>,
    pub username: Option<String>,
    pub api_key_present: bool,
    pub dedup_key: String,
    pub supported: bool,
    pub requires_password_override: bool,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportIndexerPayload {
    pub source_keys: Vec<String>,
    pub name: String,
    pub implementation: String,
    pub scryer_provider_type: Option<String>,
    pub base_url: Option<String>,
    pub api_key_present: bool,
    pub dedup_key: String,
    pub supported: bool,
    pub child_count: i32,
    pub child_names: Vec<String>,
    pub requires_api_key_override: bool,
    pub api_key_help_url: Option<String>,
}

#[derive(SimpleObject, Clone)]
pub struct ExternalImportResultPayload {
    pub media_paths_saved: bool,
    pub download_clients_created: i32,
    pub indexers_created: i32,
    pub plugins_installed: Vec<String>,
    pub errors: Vec<String>,
}

// ── Post-Processing Scripts ────────────────────────────────────────────────

#[derive(SimpleObject, Clone)]
pub struct PostProcessingScriptPayload {
    pub id: ID,
    pub name: String,
    pub description: String,
    pub script_type: String,
    pub script_content: String,
    pub applied_facets: Vec<String>,
    pub execution_mode: ExecutionModeValue,
    pub timeout_secs: i32,
    pub priority: i32,
    pub enabled: bool,
    pub debug: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct DeletePostProcessingScriptPayload {
    pub id: ID,
}

#[derive(SimpleObject, Clone)]
pub struct PostProcessingScriptRunPayload {
    pub id: ID,
    pub script_id: ID,
    pub script_name: String,
    pub title_id: Option<ID>,
    pub title_name: Option<String>,
    pub facet: Option<MediaFacetValue>,
    pub file_path: Option<String>,
    pub status: String,
    pub exit_code: Option<i32>,
    pub stdout_tail: Option<String>,
    pub stderr_tail: Option<String>,
    pub duration_ms: Option<i32>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(InputObject)]
pub struct CreatePostProcessingScriptInput {
    pub name: String,
    pub description: Option<String>,
    pub script_type: String,
    pub script_content: Option<String>,
    pub inline_shell_acknowledged: Option<bool>,
    pub applied_facets: Option<Vec<String>>,
    pub execution_mode: Option<ExecutionModeValue>,
    pub timeout_secs: Option<i32>,
    pub priority: Option<i32>,
    pub debug: Option<bool>,
}

#[derive(InputObject)]
pub struct UpdatePostProcessingScriptInput {
    pub id: ID,
    pub name: Option<String>,
    pub description: Option<String>,
    pub script_type: Option<String>,
    pub script_content: Option<String>,
    pub inline_shell_acknowledged: Option<bool>,
    pub applied_facets: Option<Vec<String>>,
    pub execution_mode: Option<ExecutionModeValue>,
    pub timeout_secs: Option<i32>,
    pub priority: Option<i32>,
    pub enabled: Option<bool>,
    pub debug: Option<bool>,
}

// ── Subtitle downloads ──────────────────────────────────────────────────────

#[derive(async_graphql::SimpleObject)]
pub struct ExternalSubtitlePayload {
    pub id: ID,
    pub media_file_id: ID,
    pub title_id: ID,
    pub episode_id: Option<ID>,
    pub source_kind: String,
    pub language: String,
    pub provider: Option<String>,
    pub provider_file_id: Option<String>,
    pub file_path: String,
    pub score: Option<i32>,
    pub score_percent: Option<i32>,
    pub hearing_impaired: bool,
    pub forced: bool,
    pub ai_translated: bool,
    pub machine_translated: bool,
    pub uploader: Option<String>,
    pub release_info: Option<String>,
    pub synced: bool,
    pub downloaded_at: DateTime<Utc>,
}

#[derive(async_graphql::SimpleObject)]
pub struct ExternalSubtitleBlocklistEntryPayload {
    pub id: ID,
    pub media_file_id: ID,
    pub provider: String,
    pub provider_file_id: String,
    pub language: String,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Title History
// ---------------------------------------------------------------------------

#[derive(SimpleObject, Clone)]
pub struct TitleHistoryEventPayload {
    pub id: ID,
    pub title_id: ID,
    pub title_name: Option<String>,
    pub facet: Option<MediaFacetValue>,
    pub episode_id: Option<ID>,
    pub episode_ids: Vec<ID>,
    pub collection_id: Option<ID>,
    pub event_type: String,
    pub actor_kind: Option<ActorKindValue>,
    pub actor_user_id: Option<ID>,
    pub actor_display_name: Option<String>,
    pub source_title: Option<String>,
    pub display_title: Option<String>,
    pub source_system: Option<String>,
    pub source_ref: Option<String>,
    pub source_provider: Option<String>,
    pub source_hint: Option<String>,
    pub quality: Option<String>,
    pub download_id: Option<String>,
    pub client_id: Option<ID>,
    pub client_name: Option<String>,
    pub import_id: Option<ID>,
    pub skip_reason: Option<String>,
    pub retry_requires_password: bool,
    pub failure_reason: Option<String>,
    pub blocklist_reason: Option<String>,
    pub source_path: Option<String>,
    pub dest_path: Option<String>,
    pub data_json: Option<Json<serde_json::Value>>,
    pub occurred_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(SimpleObject, Clone)]
pub struct TitleHistoryPagePayload {
    pub items: Vec<TitleHistoryEventPayload>,
    pub total_count: i64,
    pub has_more: bool,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(rename_items = "SCREAMING_SNAKE_CASE")]
pub enum TitleHistoryEventTypeValue {
    Requested,
    Grabbed,
    DownloadFailed,
    Blocklisted,
    DownloadCompleted,
    Imported,
    ImportFailed,
    ImportSkipped,
    FileUpgraded,
    FileRecycled,
    FileDeleted,
    FileRenamed,
    DownloadIgnored,
    Rematched,
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
            Self::DownloadIgnored => TitleHistoryEventType::DownloadIgnored,
            Self::Rematched => TitleHistoryEventType::Rematched,
        }
    }
}

#[derive(InputObject)]
pub struct TitleHistoryFilterInput {
    pub event_types: Option<Vec<TitleHistoryEventTypeValue>>,
    pub title_ids: Option<Vec<ID>>,
    pub library_ids: Option<Vec<ID>>,
    pub title_search: Option<String>,
    pub download_id: Option<String>,
    pub episode_id: Option<ID>,
    pub group_by_event: Option<bool>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}
