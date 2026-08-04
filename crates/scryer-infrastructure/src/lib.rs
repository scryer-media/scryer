#![allow(unused_imports)]

mod customization;
mod discovery;
mod downloads;
mod indexers;
mod media;
mod metadata;
mod notifications;
mod oauth;
pub mod security;
mod settings;
pub mod storage;
mod upstream_scheduler;
mod users;
mod workflow;

#[cfg(test)]
mod tests;

pub(crate) mod backup_import_normalization {
    pub(crate) use crate::workflow::backup_import_normalization::*;
}

pub(crate) mod config_store {
    pub(crate) use crate::settings::crypto::*;
}

pub(crate) mod datastore {
    pub(crate) use crate::storage::assembly::*;
}

pub(crate) mod download_client_config_store {
    pub(crate) use crate::downloads::config_store::*;
}

pub(crate) mod download_clients {
    pub(crate) use crate::downloads::clients::weaver;
    pub(crate) use crate::downloads::clients::weaver_graphql;
    pub use crate::downloads::clients::{
        BuiltinDownloadClientConnectionTester, NzbgetDownloadClient,
        PrioritizedDownloadClientRouter, SabnzbdDownloadClient, WeaverDownloadClient,
        WeaverSubscriptionBridgeClient, resolve_base_url_from_config_json,
        start_weaver_bridge_supervisor, start_weaver_subscription_bridge,
    };
    pub use crate::indexers::search_client::MultiIndexerSearchClient;
}

pub mod encryption {
    pub use crate::security::encryption::*;
}

pub mod external_identity {
    pub use crate::security::external_identity::*;
}

pub mod external_import {
    pub use scryer_application::external_import::*;
}

pub(crate) mod file_importer {
    pub(crate) use crate::workflow::file_importer::*;
}

pub(crate) mod graphql {
    pub(crate) use crate::downloads::clients::weaver_graphql as weaver;
    pub(crate) use crate::metadata::gateway::graphql as metadata_gateway;
}

pub(crate) mod indexer_config_store {
    pub(crate) use crate::indexers::config_store::*;
}

pub(crate) mod indexer_stats {
    pub(crate) use crate::indexers::stats::*;
}

pub mod indexer_caps {
    pub use crate::indexers::caps::DirectNabCapsSnapshotRefresher;
}

pub mod keystore {
    pub use crate::security::keystore::*;
}

pub(crate) mod library_renamer {
    pub(crate) use crate::media::libraries::renamer::*;
}

pub(crate) mod library_scan_unmatched_store {
    pub(crate) use crate::media::libraries::scan_unmatched_store::*;
}

pub(crate) mod library_scanner {
    pub(crate) use crate::media::libraries::scanner::*;
}

pub(crate) mod library_state_store {
    pub(crate) use crate::media::libraries::state_store::*;
}

pub(crate) mod library_store {
    pub(crate) use crate::media::libraries::store::*;
}

pub(crate) mod media_file_store {
    pub(crate) use crate::media::search::media_file_store::*;
}

pub(crate) mod media_request_store {
    pub(crate) use crate::media::requests::*;
}

pub(crate) mod media_server_connection_store {
    pub(crate) use crate::media::servers::*;
}

pub(crate) mod metadata_gateway {
    pub(crate) use crate::metadata::gateway::client::*;
}

pub mod migration_assets {
    pub use crate::storage::migrations::assets::*;
}

pub(crate) mod migration_hook_ids {
    pub(crate) use crate::storage::migrations::hook_ids::*;
}

pub mod migrations {
    pub use crate::storage::migrations::*;
}

pub mod upstream_scheduling {
    pub use crate::upstream_scheduler::InMemoryUpstreamScheduler;
}

pub(crate) mod notification_store {
    pub(crate) use crate::notifications::store::*;
}

pub(crate) mod plugin_store {
    pub(crate) use crate::customization::plugin_store::*;
}

pub(crate) mod post_processing_script_store {
    pub(crate) use crate::customization::post_processing_script_store::*;
}

pub mod postgres {
    pub(crate) use crate::storage::postgres::timestamp;
    pub use crate::storage::postgres::*;
}

pub(crate) mod prowlarr {
    pub(crate) use crate::indexers::providers::prowlarr::*;
}

pub(crate) mod queries {
    pub(crate) use crate::indexers::db as indexer;
    pub(crate) use crate::media::search::title_search;
    pub(crate) use crate::media::search::wanted;
    pub(crate) use crate::media::titles::db as title;
    pub(crate) use crate::storage::sql::common;
    pub(crate) use crate::storage::sql::runtime as sql_runtime;
}

pub(crate) mod quality_profile_store {
    pub(crate) use crate::settings::quality_profile_store::*;
}

pub(crate) mod release_store {
    pub(crate) use crate::workflow::release_store::*;
}

pub(crate) mod rule_set_store {
    pub(crate) use crate::customization::rule_set_store::*;
}

pub(crate) mod settings_store {
    pub(crate) use crate::settings::settings_store::*;
}

pub(crate) mod show_store {
    pub(crate) use crate::media::shows::store::*;
}

pub mod smg_enrollment {
    pub use crate::metadata::enrollment::*;
}

pub(crate) mod spellfix {
    pub(crate) use crate::storage::sql::spellfix::*;
}

pub(crate) mod sqlite_backup {
    pub(crate) use crate::storage::sqlite::backup::*;
}

pub(crate) mod sqlite_services {
    pub(crate) use crate::storage::sqlite::services::*;
}

pub(crate) mod staged_nzb_store {
    pub(crate) use crate::downloads::staged_nzb_store::*;
}

pub(crate) mod subtitle_provider_config_store {
    pub(crate) use crate::settings::subtitle_provider_config_store::*;
}

pub(crate) mod title_image_store {
    pub(crate) use crate::media::images::title_image_store::*;
}

pub(crate) mod title_images {
    #[cfg(feature = "image-processing")]
    pub(crate) use crate::media::images::processor::*;
    pub(crate) use crate::media::images::{content_type_for_format, normalized_base_path_from_env};
}

pub(crate) mod title_store {
    pub(crate) use crate::media::titles::store::*;
}

pub(crate) mod types {
    pub(crate) use crate::storage::types::*;
}

pub(crate) mod user_store {
    pub(crate) use crate::users::store::*;
}

pub(crate) mod webauthn_store {
    pub(crate) use crate::users::webauthn_store::*;
}

pub(crate) mod workflow_store {
    pub(crate) use crate::workflow::stores::*;
}

pub mod sqlite {
    pub use crate::customization::plugin_store::PluginStore;
    pub use crate::customization::post_processing_script_store::PostProcessingScriptStore;
    pub use crate::customization::rule_set_store::RuleSetStore;
    pub use crate::downloads::config_store::DownloadClientConfigStore;
    pub use crate::indexers::config_store::IndexerConfigStore;
    pub use crate::indexers::proxy_config_store::IndexerProxyConfigStore;
    pub use crate::indexers::scope_indexer_coverage_store::ScopeIndexerCoverageStore;
    #[cfg(feature = "image-processing")]
    pub use crate::media::images::processor::HttpTitleImageProcessor;
    pub use crate::media::images::title_image_store::TitleImageStore;
    pub use crate::media::libraries::scan_unmatched_store::LibraryScanUnmatchedStore;
    pub use crate::media::libraries::state_store::{
        BlocklistStore, HousekeepingStore, LibraryProbeStore, PendingReleaseStore,
        SubtitleDownloadStore, WantedStore,
    };
    pub use crate::media::libraries::store::LibraryStore;
    pub use crate::media::requests::MediaRequestStore;
    pub use crate::media::search::media_file_store::MediaFileStore;
    pub use crate::media::servers::MediaServerConnectionStore;
    pub use crate::media::shows::store::ShowStore;
    pub use crate::media::titles::store::TitleStore;
    pub use crate::notifications::store::NotificationStore;
    pub use crate::oauth::store::OAuthStore;
    pub use crate::settings::quality_profile_store::QualityProfileStore;
    pub use crate::settings::settings_store::SettingsStore;
    pub use crate::settings::subtitle_provider_config_store::SubtitleProviderConfigStore;
    pub use crate::storage::sqlite::backup::SqliteLogicalBackupExporter;
    pub use crate::storage::sqlite::services::SqliteServices;
    pub use crate::users::store::UserStore;
    pub use crate::workflow::release_store::ReleaseStore;
    pub use crate::workflow::stores::{
        AcquisitionStore, DomainEventStore, DownloadQueueCommandStore, DownloadSubmissionStore,
        ExternalImportMonitorStore, ExternalImportSetupSecretDraftStore, ImportStore,
        WorkflowOperationStore,
    };
}

pub use customization::plugin_store::PluginStore;
pub use customization::post_processing_script_store::PostProcessingScriptStore;
pub use customization::rule_set_store::RuleSetStore;
pub use downloads::clients::{
    BuiltinDownloadClientConnectionTester, NzbgetDownloadClient, PrioritizedDownloadClientRouter,
    SabnzbdDownloadClient, WeaverDownloadClient, WeaverSubscriptionBridgeClient,
    resolve_base_url_from_config_json, start_weaver_bridge_supervisor,
    start_weaver_subscription_bridge,
};
pub use downloads::config_store::DownloadClientConfigStore;
pub use downloads::staged_nzb_store::FileSystemStagedNzbStore;
pub use indexers::config_store::IndexerConfigStore;
pub use indexers::providers::prowlarr::{NativeProwlarrIndexerProvider, PROWLARR_PROVIDER_TYPE};
pub use indexers::proxy_config_store::IndexerProxyConfigStore;
pub use indexers::scope_indexer_coverage_store::ScopeIndexerCoverageStore;
pub use indexers::search_client::MultiIndexerSearchClient;
pub use indexers::search_learning::IndexerSearchLearningStore;
pub use indexers::stats::InMemoryIndexerStatsTracker;
#[cfg(feature = "image-processing")]
pub use media::images::processor::HttpTitleImageProcessor;
pub use media::images::title_image_store::TitleImageStore;
pub use media::images::{ImageProxyBlob, ImageProxyRuntime, ImageProxyStore};
pub use media::libraries::renamer::FileSystemLibraryRenamer;
pub use media::libraries::scan_unmatched_store::LibraryScanUnmatchedStore;
pub use media::libraries::scanner::FileSystemLibraryScanner;
pub use media::libraries::state_store::{
    BlocklistStore, HousekeepingStore, LibraryProbeStore, PendingReleaseStore,
    SubtitleDownloadStore, WantedStore,
};
pub use media::libraries::store::LibraryStore;
pub use media::requests::MediaRequestStore;
pub use media::search::media_file_store::MediaFileStore;
pub use media::servers::MediaServerConnectionStore;
pub use media::shows::store::ShowStore;
pub use media::titles::store::TitleStore;
pub use metadata::gateway::client::{MetadataGatewayClient, SmgEnrollmentConfig};
pub use notifications::store::NotificationStore;
pub use oauth::store::OAuthStore;
pub use security::encryption::EncryptionKey;
pub use settings::quality_profile_store::QualityProfileStore;
pub use settings::settings_store::SettingsStore;
pub use settings::subtitle_provider_config_store::SubtitleProviderConfigStore;
pub use storage::assembly::{
    DatastoreAssembly, DatastoreConfig, DatastoreConfigSource, DatastoreCustomizationStore,
    DatastoreEncryptionBootstrapReport, DatastoreEngine, datastore_file_path,
    resolve_datastore_config_from_env, restore_backup_bundle_to_datastore,
    restore_backup_bundle_to_datastore_path, restore_prepared_backup_directory_to_datastore,
    validate_datastore,
};
pub use storage::migrations::{list_embedded_migration_keys, list_embedded_migrations};
pub use storage::postgres::{PostgresLogicalBackupExporter, PostgresServices};
pub use storage::sql::spellfix::register_spellfix_auto_extension;
pub use storage::sqlite::backup::SqliteLogicalBackupExporter;
pub use storage::sqlite::services::SqliteServices;
pub(crate) use storage::types::sqlite_url_with_create;
pub use storage::types::{
    DownloadQueueCommandRecord, EmbeddedMigrationDescriptor, LibraryProbeSignatureRecord,
    MigrationMode, MigrationStatus, SettingDefinitionSeed, SettingsDefinitionRecord,
    SettingsValueRecord, WorkflowOperationRecord,
};
pub use users::store::UserStore;
pub use users::totp_store::TotpStore;
pub use users::webauthn_store::WebauthnStore;
pub use workflow::file_importer::FsFileImporter;
pub use workflow::release_store::ReleaseStore;
pub use workflow::stores::{
    AcquisitionStore, DomainEventStore, DownloadQueueCommandStore, DownloadSubmissionStore,
    ExternalImportMonitorStore, ExternalImportSetupSecretDraftStore, ImportStore,
    WorkflowOperationStore,
};
