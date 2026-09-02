mod application_upgrade;
mod collections;
mod config;
mod interactive_search;
mod library;
mod location;
mod maintenance_rules;
mod notifications;
mod recycle_bin;
mod rules;
mod subtitle;
mod titles;

use scryer_interface_acquisition::{
    DownloadMutations, JobMutations, MediaRequestMutations, PostProcessingMutations,
    WantedMutations,
};
use scryer_interface_import::ExternalImportMutations;
use scryer_interface_security::UserMutations;
use scryer_interface_system::{BackupMutations, PluginMutations};

use async_graphql::MergedObject;
use scryer_interface_settings::SettingsMutations;

#[derive(MergedObject, Default)]
pub struct MutationRoot(
    application_upgrade::ApplicationUpgradeMutations,
    titles::TitleMutations,
    collections::CollectionMutations,
    DownloadMutations,
    JobMutations,
    config::ConfigMutations,
    SettingsMutations,
    UserMutations,
    library::LibraryMutations,
    location::LocationMutations,
    MediaRequestMutations,
    WantedMutations,
    rules::RulesMutations,
    maintenance_rules::MaintenanceRuleMutations,
    PluginMutations,
    notifications::NotificationMutations,
    BackupMutations,
    ExternalImportMutations,
    PostProcessingMutations,
    subtitle::SubtitleMutations,
    recycle_bin::RecycleBinMutations,
    interactive_search::InteractiveSearchMutations,
);
