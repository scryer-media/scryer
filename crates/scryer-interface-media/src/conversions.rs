use crate::types::*;
use scryer_application::{
    AcquisitionScopeStatus as AppWantedStatus, ActivityChannel as AppActivityChannel,
    ActivityKind as AppActivityKind, ActivitySeverity as AppActivitySeverity,
    DownloadHistorySortKey as AppDownloadHistorySortKey,
    DownloadSourceKind as AppDownloadSourceKind,
    DownloadSubmissionPurpose as AppDownloadSubmissionPurpose, JobCategory as AppJobCategory,
    JobKey as AppJobKey, JobRunStatus as AppJobRunStatus, JobScheduleKind as AppJobScheduleKind,
    JobSection as AppJobSection, JobTriggerSource as AppJobTriggerSource,
    LibraryScanMode as AppLibraryScanMode, LibraryScanStatus as AppLibraryScanStatus,
    PendingImportReasonClass as AppPendingImportReasonClass,
    PendingImportStatus as AppPendingImportStatus, PendingReleaseRole as AppPendingReleaseRole,
    PendingReleaseStatus as AppPendingReleaseStatus, ScoringOverrides as AppScoringOverrides,
    ScoringPersona as AppScoringPersona, SortDirection as AppSortDirection,
    SubmissionScope as AppSubmissionScope,
};

pub trait FromApplication<T> {
    fn from_application(value: T) -> Self;
}

pub trait IntoApplication<T> {
    fn into_application(self) -> T;
}

impl IntoApplication<AppPendingImportStatus> for PendingImportStatusValue {
    fn into_application(self) -> AppPendingImportStatus {
        match self {
            Self::Pending => AppPendingImportStatus::Pending,
            Self::Ignored => AppPendingImportStatus::Ignored,
        }
    }
}

impl FromApplication<AppPendingImportStatus> for PendingImportStatusValue {
    fn from_application(value: AppPendingImportStatus) -> Self {
        match value {
            AppPendingImportStatus::Pending => Self::Pending,
            AppPendingImportStatus::Ignored => Self::Ignored,
        }
    }
}

impl FromApplication<AppPendingImportReasonClass> for PendingImportReasonClassValue {
    fn from_application(value: AppPendingImportReasonClass) -> Self {
        match value {
            AppPendingImportReasonClass::Unmatched => Self::Unmatched,
            AppPendingImportReasonClass::Ambiguous => Self::Ambiguous,
            AppPendingImportReasonClass::QualityUnknown => Self::QualityUnknown,
            AppPendingImportReasonClass::Other => Self::Other,
        }
    }
}

impl FromApplication<AppScoringPersona> for ScoringPersonaValue {
    fn from_application(value: AppScoringPersona) -> Self {
        match value {
            AppScoringPersona::Balanced => Self::Balanced,
            AppScoringPersona::Audiophile => Self::Audiophile,
            AppScoringPersona::Efficient => Self::Efficient,
            AppScoringPersona::Compatible => Self::Compatible,
        }
    }
}

impl IntoApplication<AppScoringPersona> for ScoringPersonaValue {
    fn into_application(self) -> AppScoringPersona {
        match self {
            Self::Balanced => AppScoringPersona::Balanced,
            Self::Audiophile => AppScoringPersona::Audiophile,
            Self::Efficient => AppScoringPersona::Efficient,
            Self::Compatible => AppScoringPersona::Compatible,
        }
    }
}

impl IntoApplication<AppDownloadSourceKind> for DownloadSourceKindValue {
    fn into_application(self) -> AppDownloadSourceKind {
        match self {
            Self::NzbFile => AppDownloadSourceKind::NzbFile,
            Self::NzbUrl => AppDownloadSourceKind::NzbUrl,
            Self::TorrentFile => AppDownloadSourceKind::TorrentFile,
            Self::MagnetUri => AppDownloadSourceKind::MagnetUri,
        }
    }
}

impl FromApplication<AppDownloadSourceKind> for DownloadSourceKindValue {
    fn from_application(value: AppDownloadSourceKind) -> Self {
        match value {
            AppDownloadSourceKind::NzbFile => Self::NzbFile,
            AppDownloadSourceKind::NzbUrl => Self::NzbUrl,
            AppDownloadSourceKind::TorrentFile => Self::TorrentFile,
            AppDownloadSourceKind::MagnetUri => Self::MagnetUri,
        }
    }
}

impl IntoApplication<AppDownloadSubmissionPurpose> for QueueDownloadPurposeValue {
    fn into_application(self) -> AppDownloadSubmissionPurpose {
        match self {
            Self::Standard => AppDownloadSubmissionPurpose::OperatorQueued,
            Self::AdditionalFile => AppDownloadSubmissionPurpose::AdditionalFile,
        }
    }
}

impl IntoApplication<scryer_application::PreferredProtocol> for DelayProfilePreferredProtocolValue {
    fn into_application(self) -> scryer_application::PreferredProtocol {
        match self {
            Self::Usenet => scryer_application::PreferredProtocol::Usenet,
            Self::Torrent => scryer_application::PreferredProtocol::Torrent,
        }
    }
}

impl FromApplication<scryer_application::PreferredProtocol> for DelayProfilePreferredProtocolValue {
    fn from_application(value: scryer_application::PreferredProtocol) -> Self {
        match value {
            scryer_application::PreferredProtocol::Usenet => Self::Usenet,
            scryer_application::PreferredProtocol::Torrent => Self::Torrent,
        }
    }
}

impl FromApplication<scryer_application::DownloadDisplayState> for DownloadDisplayStateValue {
    fn from_application(value: scryer_application::DownloadDisplayState) -> Self {
        match value {
            scryer_application::DownloadDisplayState::Queued => Self::Queued,
            scryer_application::DownloadDisplayState::Downloading => Self::Downloading,
            scryer_application::DownloadDisplayState::Paused => Self::Paused,
            scryer_application::DownloadDisplayState::PostProcessing => Self::PostProcessing,
            scryer_application::DownloadDisplayState::Completed => Self::Completed,
            scryer_application::DownloadDisplayState::ImportedSeeding => Self::ImportedSeeding,
            scryer_application::DownloadDisplayState::Failed => Self::Failed,
            scryer_application::DownloadDisplayState::Warning => Self::Warning,
            scryer_application::DownloadDisplayState::Importing => Self::Importing,
            scryer_application::DownloadDisplayState::ImportPending => Self::ImportPending,
            scryer_application::DownloadDisplayState::ImportBlocked => Self::ImportBlocked,
            scryer_application::DownloadDisplayState::ImportFailed => Self::ImportFailed,
            scryer_application::DownloadDisplayState::Ignored => Self::Ignored,
            scryer_application::DownloadDisplayState::Removing => Self::Removing,
            scryer_application::DownloadDisplayState::RemoveFailed => Self::RemoveFailed,
        }
    }
}

impl FromApplication<scryer_application::DownloadSeedingState> for DownloadSeedingStateValue {
    fn from_application(value: scryer_application::DownloadSeedingState) -> Self {
        match value {
            scryer_application::DownloadSeedingState::None => Self::None,
            scryer_application::DownloadSeedingState::Seeding => Self::Seeding,
            scryer_application::DownloadSeedingState::GoalMet => Self::GoalMet,
            scryer_application::DownloadSeedingState::HeldPrivate => Self::HeldPrivate,
            scryer_application::DownloadSeedingState::NeverRemove => Self::NeverRemove,
        }
    }
}

impl IntoApplication<scryer_application::DownloadActivityFilter> for DownloadActivityFilterValue {
    fn into_application(self) -> scryer_application::DownloadActivityFilter {
        match self {
            Self::All => scryer_application::DownloadActivityFilter::All,
            Self::Downloading => scryer_application::DownloadActivityFilter::Downloading,
            Self::Queued => scryer_application::DownloadActivityFilter::Queued,
            Self::Paused => scryer_application::DownloadActivityFilter::Paused,
            Self::PostProcessing => scryer_application::DownloadActivityFilter::PostProcessing,
            Self::Seeding => scryer_application::DownloadActivityFilter::Seeding,
            Self::Warning => scryer_application::DownloadActivityFilter::Warning,
        }
    }
}

impl IntoApplication<scryer_application::DownloadImportFilter> for DownloadImportFilterValue {
    fn into_application(self) -> scryer_application::DownloadImportFilter {
        match self {
            Self::All => scryer_application::DownloadImportFilter::All,
            Self::Attention => scryer_application::DownloadImportFilter::Attention,
            Self::Importing => scryer_application::DownloadImportFilter::Importing,
            Self::Pending => scryer_application::DownloadImportFilter::Pending,
            Self::Blocked => scryer_application::DownloadImportFilter::Blocked,
            Self::Failed => scryer_application::DownloadImportFilter::Failed,
        }
    }
}

impl IntoApplication<scryer_application::DownloadHistoryFilter> for DownloadHistoryFilterValue {
    fn into_application(self) -> scryer_application::DownloadHistoryFilter {
        match self {
            Self::All => scryer_application::DownloadHistoryFilter::All,
            Self::Success => scryer_application::DownloadHistoryFilter::Success,
            Self::Failed => scryer_application::DownloadHistoryFilter::Failed,
        }
    }
}

impl IntoApplication<AppDownloadHistorySortKey> for DownloadHistorySortKeyValue {
    fn into_application(self) -> AppDownloadHistorySortKey {
        match self {
            Self::Title => AppDownloadHistorySortKey::Title,
            Self::Client => AppDownloadHistorySortKey::Client,
            Self::Status => AppDownloadHistorySortKey::Status,
            Self::Progress => AppDownloadHistorySortKey::Progress,
            Self::Size => AppDownloadHistorySortKey::Size,
        }
    }
}

impl IntoApplication<AppDownloadHistorySortKey> for DownloadQueueSortKeyValue {
    fn into_application(self) -> AppDownloadHistorySortKey {
        match self {
            Self::Title => AppDownloadHistorySortKey::Title,
            Self::Client => AppDownloadHistorySortKey::Client,
            Self::Status => AppDownloadHistorySortKey::Status,
            Self::Progress => AppDownloadHistorySortKey::Progress,
            Self::Size => AppDownloadHistorySortKey::Size,
        }
    }
}

impl IntoApplication<AppSortDirection> for SortDirectionValue {
    fn into_application(self) -> AppSortDirection {
        match self {
            Self::Asc => AppSortDirection::Asc,
            Self::Desc => AppSortDirection::Desc,
        }
    }
}

impl FromApplication<AppActivityKind> for ActivityKindValue {
    fn from_application(value: AppActivityKind) -> Self {
        match value {
            AppActivityKind::SettingSaved => Self::SettingSaved,
            AppActivityKind::MovieFetched => Self::MovieFetched,
            AppActivityKind::TitleAdded => Self::TitleAdded,
            AppActivityKind::TitleUpdated => Self::TitleUpdated,
            AppActivityKind::MetadataHydrationStarted => Self::MetadataHydrationStarted,
            AppActivityKind::MetadataHydrationCompleted => Self::MetadataHydrationCompleted,
            AppActivityKind::MetadataHydrationFailed => Self::MetadataHydrationFailed,
            AppActivityKind::MovieDownloaded => Self::MovieDownloaded,
            AppActivityKind::SeriesEpisodeImported => Self::SeriesEpisodeImported,
            AppActivityKind::AcquisitionSearchCompleted => Self::AcquisitionSearchCompleted,
            AppActivityKind::AcquisitionCandidateAccepted => Self::AcquisitionCandidateAccepted,
            AppActivityKind::AcquisitionCandidateRejected => Self::AcquisitionCandidateRejected,
            AppActivityKind::AcquisitionDownloadFailed => Self::AcquisitionDownloadFailed,
            AppActivityKind::PostProcessingCompleted => Self::PostProcessingCompleted,
            AppActivityKind::FileAnalyzed => Self::FileAnalyzed,
            AppActivityKind::FileUpgraded => Self::FileUpgraded,
            AppActivityKind::ImportRejected => Self::ImportRejected,
            AppActivityKind::SubtitleDownloaded => Self::SubtitleDownloaded,
            AppActivityKind::SubtitleSearchFailed => Self::SubtitleSearchFailed,
            AppActivityKind::SystemNotice => Self::SystemNotice,
        }
    }
}

impl FromApplication<AppActivitySeverity> for ActivitySeverityValue {
    fn from_application(value: AppActivitySeverity) -> Self {
        match value {
            AppActivitySeverity::Info => Self::Info,
            AppActivitySeverity::Success => Self::Success,
            AppActivitySeverity::Warning => Self::Warning,
            AppActivitySeverity::Error => Self::Error,
        }
    }
}

impl FromApplication<AppActivityChannel> for ActivityChannelValue {
    fn from_application(value: AppActivityChannel) -> Self {
        match value {
            AppActivityChannel::WebUi => Self::WebUi,
            AppActivityChannel::Toast => Self::Toast,
        }
    }
}

impl FromApplication<AppWantedStatus> for WantedStatusValue {
    fn from_application(value: AppWantedStatus) -> Self {
        match value {
            AppWantedStatus::Wanted => Self::Wanted,
            AppWantedStatus::Grabbed => Self::Grabbed,
            AppWantedStatus::Paused => Self::Paused,
            AppWantedStatus::Completed => Self::Completed,
        }
    }
}

impl IntoApplication<AppWantedStatus> for WantedStatusValue {
    fn into_application(self) -> AppWantedStatus {
        match self {
            Self::Wanted => AppWantedStatus::Wanted,
            Self::Grabbed => AppWantedStatus::Grabbed,
            Self::Paused => AppWantedStatus::Paused,
            Self::Completed => AppWantedStatus::Completed,
        }
    }
}

impl FromApplication<AppPendingReleaseStatus> for PendingReleaseStatusValue {
    fn from_application(value: AppPendingReleaseStatus) -> Self {
        match value {
            AppPendingReleaseStatus::Waiting => Self::Waiting,
            AppPendingReleaseStatus::Standby => Self::Standby,
            AppPendingReleaseStatus::Processing => Self::Processing,
            AppPendingReleaseStatus::Grabbed => Self::Grabbed,
            AppPendingReleaseStatus::Superseded => Self::Superseded,
            AppPendingReleaseStatus::Expired => Self::Expired,
            AppPendingReleaseStatus::Dismissed => Self::Dismissed,
            AppPendingReleaseStatus::NeedsReview => Self::NeedsReview,
        }
    }
}

impl IntoApplication<AppPendingReleaseStatus> for PendingReleaseStatusValue {
    fn into_application(self) -> AppPendingReleaseStatus {
        match self {
            Self::Waiting => AppPendingReleaseStatus::Waiting,
            Self::Standby => AppPendingReleaseStatus::Standby,
            Self::Processing => AppPendingReleaseStatus::Processing,
            Self::Grabbed => AppPendingReleaseStatus::Grabbed,
            Self::Superseded => AppPendingReleaseStatus::Superseded,
            Self::Expired => AppPendingReleaseStatus::Expired,
            Self::Dismissed => AppPendingReleaseStatus::Dismissed,
            Self::NeedsReview => AppPendingReleaseStatus::NeedsReview,
        }
    }
}

impl FromApplication<AppPendingReleaseRole> for PendingReleaseRoleValue {
    fn from_application(value: AppPendingReleaseRole) -> Self {
        match value {
            AppPendingReleaseRole::Primary => Self::Primary,
            AppPendingReleaseRole::Fallback => Self::Fallback,
        }
    }
}

impl IntoApplication<AppJobKey> for JobKeyValue {
    fn into_application(self) -> AppJobKey {
        match self {
            Self::LibraryScanMovies => AppJobKey::LibraryScanMovies,
            Self::LibraryScanSeries => AppJobKey::LibraryScanSeries,
            Self::LibraryScanAnime => AppJobKey::LibraryScanAnime,
            Self::BackgroundLibraryRefreshMovies => AppJobKey::BackgroundLibraryRefreshMovies,
            Self::BackgroundLibraryRefreshSeries => AppJobKey::BackgroundLibraryRefreshSeries,
            Self::BackgroundLibraryRefreshAnime => AppJobKey::BackgroundLibraryRefreshAnime,
            Self::RssSync => AppJobKey::RssSync,
            Self::SubtitleSearch => AppJobKey::SubtitleSearch,
            Self::PluginRegistryRefresh => AppJobKey::PluginRegistryRefresh,
            Self::Housekeeping => AppJobKey::Housekeeping,
            Self::HealthChecks => AppJobKey::HealthChecks,
            Self::AutoBackup => AppJobKey::AutoBackup,
            Self::ProwlarrSync => AppJobKey::ProwlarrSync,
            Self::PendingReleaseProcessing => AppJobKey::PendingReleaseProcessing,
            Self::StagedNzbPrune => AppJobKey::StagedNzbPrune,
            Self::DiscoverySync => AppJobKey::DiscoverySync,
            Self::TitleImageCacheRefresh => AppJobKey::TitleImageCacheRefresh,
            Self::MaintenanceRuleEvaluation => AppJobKey::MaintenanceRuleEvaluation,
            Self::LifecycleActionHandling => AppJobKey::LifecycleActionHandling,
            Self::TitleDeletion => AppJobKey::TitleDeletion,
            Self::TitleRename => AppJobKey::TitleRename,
            Self::MediaFileDeletion => AppJobKey::MediaFileDeletion,
            Self::RecycleBinRestore => AppJobKey::RecycleBinRestore,
            Self::RecycleBinPurge => AppJobKey::RecycleBinPurge,
            Self::AcquisitionSearch => AppJobKey::AcquisitionSearch,
            Self::ApplicationUpgrade => AppJobKey::ApplicationUpgrade,
        }
    }
}

impl FromApplication<AppJobKey> for JobKeyValue {
    fn from_application(value: AppJobKey) -> Self {
        match value {
            AppJobKey::LibraryScanMovies => Self::LibraryScanMovies,
            AppJobKey::LibraryScanSeries => Self::LibraryScanSeries,
            AppJobKey::LibraryScanAnime => Self::LibraryScanAnime,
            AppJobKey::BackgroundLibraryRefreshMovies => Self::BackgroundLibraryRefreshMovies,
            AppJobKey::BackgroundLibraryRefreshSeries => Self::BackgroundLibraryRefreshSeries,
            AppJobKey::BackgroundLibraryRefreshAnime => Self::BackgroundLibraryRefreshAnime,
            AppJobKey::RssSync => Self::RssSync,
            AppJobKey::SubtitleSearch => Self::SubtitleSearch,
            AppJobKey::PluginRegistryRefresh => Self::PluginRegistryRefresh,
            AppJobKey::Housekeeping => Self::Housekeeping,
            AppJobKey::HealthChecks => Self::HealthChecks,
            AppJobKey::AutoBackup => Self::AutoBackup,
            AppJobKey::ProwlarrSync => Self::ProwlarrSync,
            AppJobKey::PendingReleaseProcessing => Self::PendingReleaseProcessing,
            AppJobKey::StagedNzbPrune => Self::StagedNzbPrune,
            AppJobKey::DiscoverySync => Self::DiscoverySync,
            AppJobKey::TitleImageCacheRefresh => Self::TitleImageCacheRefresh,
            AppJobKey::MaintenanceRuleEvaluation => Self::MaintenanceRuleEvaluation,
            AppJobKey::LifecycleActionHandling => Self::LifecycleActionHandling,
            AppJobKey::TitleDeletion => Self::TitleDeletion,
            AppJobKey::TitleRename => Self::TitleRename,
            AppJobKey::MediaFileDeletion => Self::MediaFileDeletion,
            AppJobKey::RecycleBinRestore => Self::RecycleBinRestore,
            AppJobKey::RecycleBinPurge => Self::RecycleBinPurge,
            AppJobKey::AcquisitionSearch => Self::AcquisitionSearch,
            AppJobKey::ApplicationUpgrade => Self::ApplicationUpgrade,
        }
    }
}

impl FromApplication<AppJobCategory> for JobCategoryValue {
    fn from_application(value: AppJobCategory) -> Self {
        match value {
            AppJobCategory::Library => Self::Library,
            AppJobCategory::Acquisition => Self::Acquisition,
            AppJobCategory::Maintenance => Self::Maintenance,
            AppJobCategory::Subtitles => Self::Subtitles,
            AppJobCategory::System => Self::System,
        }
    }
}

impl FromApplication<AppJobSection> for JobSectionValue {
    fn from_application(value: AppJobSection) -> Self {
        match value {
            AppJobSection::Primary => Self::Primary,
            AppJobSection::Maintenance => Self::Maintenance,
        }
    }
}

impl FromApplication<AppJobScheduleKind> for JobScheduleKindValue {
    fn from_application(value: AppJobScheduleKind) -> Self {
        match value {
            AppJobScheduleKind::Manual => Self::Manual,
            AppJobScheduleKind::Interval => Self::Interval,
            AppJobScheduleKind::StartupAndInterval => Self::StartupAndInterval,
            AppJobScheduleKind::DailyAtTime => Self::DailyAtTime,
        }
    }
}

impl FromApplication<AppJobTriggerSource> for JobTriggerSourceValue {
    fn from_application(value: AppJobTriggerSource) -> Self {
        match value {
            AppJobTriggerSource::Manual => Self::Manual,
            AppJobTriggerSource::ScheduledStartup => Self::ScheduledStartup,
            AppJobTriggerSource::ScheduledInterval => Self::ScheduledInterval,
            AppJobTriggerSource::ScheduledDaily => Self::ScheduledDaily,
            AppJobTriggerSource::SystemInternal => Self::SystemInternal,
        }
    }
}

impl FromApplication<AppJobRunStatus> for JobRunStatusValue {
    fn from_application(value: AppJobRunStatus) -> Self {
        match value {
            AppJobRunStatus::Queued => Self::Queued,
            AppJobRunStatus::Discovering => Self::Discovering,
            AppJobRunStatus::Running => Self::Running,
            AppJobRunStatus::Completed => Self::Completed,
            AppJobRunStatus::Warning => Self::Warning,
            AppJobRunStatus::Failed => Self::Failed,
        }
    }
}

impl FromApplication<AppLibraryScanMode> for LibraryScanModeValue {
    fn from_application(value: AppLibraryScanMode) -> Self {
        match value {
            AppLibraryScanMode::Full => Self::Full,
            AppLibraryScanMode::Additive => Self::Additive,
        }
    }
}

impl FromApplication<AppLibraryScanStatus> for LibraryScanStatusValue {
    fn from_application(value: AppLibraryScanStatus) -> Self {
        match value {
            AppLibraryScanStatus::Discovering => Self::Discovering,
            AppLibraryScanStatus::Running => Self::Running,
            AppLibraryScanStatus::Completed => Self::Completed,
            AppLibraryScanStatus::Canceled => Self::Canceled,
            AppLibraryScanStatus::Warning => Self::Warning,
            AppLibraryScanStatus::Failed => Self::Failed,
        }
    }
}

impl IntoApplication<AppSubmissionScope> for QueueDownloadScopeInput {
    fn into_application(self) -> AppSubmissionScope {
        match self {
            Self::Episode(episode_id) => AppSubmissionScope::Episode {
                episode_id: episode_id.to_string(),
            },
            Self::EpisodeSet(episode_ids) => {
                let mut episode_ids = episode_ids
                    .into_iter()
                    .map(|episode_id| episode_id.to_string())
                    .collect::<Vec<_>>();
                episode_ids.retain(|episode_id| !episode_id.trim().is_empty());
                episode_ids.sort();
                episode_ids.dedup();
                AppSubmissionScope::EpisodeSet { episode_ids }
            }
            Self::SeriesMovie(series_movie_link_id) => AppSubmissionScope::SeriesMovie {
                series_movie_link_id: series_movie_link_id.to_string(),
            },
            Self::Collection(collection_id) => AppSubmissionScope::Collection {
                collection_id: collection_id.to_string(),
            },
            Self::Title(_) => AppSubmissionScope::Title,
        }
    }
}

impl FromApplication<scryer_application::AddTitleHydrationState> for AddTitleHydrationStateValue {
    fn from_application(value: scryer_application::AddTitleHydrationState) -> Self {
        match value {
            scryer_application::AddTitleHydrationState::Pending => Self::Pending,
            scryer_application::AddTitleHydrationState::Complete => Self::Complete,
            scryer_application::AddTitleHydrationState::NotRequired => Self::NotRequired,
        }
    }
}

impl IntoApplication<AppScoringOverrides> for ScoringOverridesInput {
    fn into_application(self) -> AppScoringOverrides {
        AppScoringOverrides {
            allow_x265_non4k: self.allow_x265_non4k,
            block_dv_without_fallback: self.block_dv_without_fallback,
            prefer_compact_encodes: self.prefer_compact_encodes,
            prefer_lossless_audio: self.prefer_lossless_audio,
            block_upscaled: self.block_upscaled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_queue_download_purposes_preserve_their_application_lanes() {
        assert_eq!(
            QueueDownloadPurposeValue::Standard.into_application(),
            AppDownloadSubmissionPurpose::OperatorQueued
        );
        assert_eq!(
            QueueDownloadPurposeValue::AdditionalFile.into_application(),
            AppDownloadSubmissionPurpose::AdditionalFile
        );
    }
}
