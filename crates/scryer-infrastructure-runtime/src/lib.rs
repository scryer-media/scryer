pub mod assembly;
pub mod backup_import_normalization;
pub mod postgres_backup;
pub mod sqlite_backup;

#[cfg(test)]
mod tests;

pub(crate) mod discovery {
    pub(crate) use scryer_infrastructure_metadata::discovery::*;
}

pub(crate) mod external_identity {
    pub(crate) use scryer_infrastructure_identity::external_identity::*;
}

/// Maintenance safety: live playback observation (RFC 137 §9.10, WP-G).
pub(crate) mod media_server_playback {
    pub(crate) use scryer_infrastructure_identity::media_server_playback::*;
}

/// Provider adapters for media-server watch signals (RFC 137 §7.3, WP-M).
pub(crate) mod media_server_signals {
    pub(crate) use scryer_infrastructure_identity::media_server_signals::*;
}

#[cfg(test)]
pub(crate) mod media_server_connection_store {
    pub(crate) use scryer_infrastructure_library::media::servers::*;
}

pub(crate) mod indexers {
    pub(crate) use scryer_infrastructure_acquisition::indexers::*;
}

pub(crate) mod media {
    pub(crate) use scryer_infrastructure_library::media::*;
}

pub(crate) mod postgres {
    pub(crate) use crate::postgres_backup::{
        PostgresLogicalBackupExporter, restore_backup_bundle_into_postgres_pool,
        restore_prepared_backup_directory_into_postgres_pool,
    };
    pub(crate) use scryer_infrastructure_datastore::postgres::PostgresServices;
    #[cfg(test)]
    pub(crate) use scryer_infrastructure_datastore::postgres::replay_source_catalog_for_fresh_install;
}

pub(crate) mod queries {
    pub(crate) use scryer_infrastructure_library_search as title_search;
    pub(crate) use scryer_infrastructure_sql::runtime as sql_runtime;
}

pub(crate) mod upstream_scheduler {
    pub(crate) use scryer_infrastructure_acquisition::upstream_scheduler::*;
}

#[cfg(test)]
pub(crate) mod workflow {
    pub(crate) use scryer_infrastructure_workflow::workflow::*;
}

#[cfg(test)]
pub(crate) mod workflow_store {
    pub(crate) use scryer_infrastructure_workflow::workflow::stores::*;
}

#[cfg(test)]
pub(crate) mod migrations {
    pub(crate) use scryer_infrastructure_datastore::migrations::*;
}

#[cfg(test)]
pub(crate) mod migration_assets {
    pub(crate) use scryer_infrastructure_datastore::migration_assets::*;
}

#[cfg(test)]
pub(crate) mod spellfix {
    pub(crate) use scryer_infrastructure_datastore::spellfix::*;
}

#[cfg(test)]
pub(crate) mod types {
    pub(crate) use scryer_infrastructure_sql::types::*;
}

pub(crate) use crate::sqlite_backup::SqliteLogicalBackupExporter;
#[cfg(test)]
pub(crate) use scryer_infrastructure_acquisition::downloads::clients::NzbgetDownloadClient;
pub(crate) use scryer_infrastructure_acquisition::downloads::{
    config_store::DownloadClientConfigStore, seeding_profile_store::SeedingProfileStore,
    staged_nzb_store::FileSystemStagedNzbStore,
};
pub(crate) use scryer_infrastructure_acquisition::indexers::{
    config_store::IndexerConfigStore, error_store::IndexerErrorStore,
    proxy_config_store::IndexerProxyConfigStore, search_learning::IndexerSearchLearningStore,
    stats::InMemoryIndexerStatsTracker,
};
pub(crate) use scryer_infrastructure_configuration::customization::{
    maintenance_evaluation_store::MaintenanceEvaluationStore,
    maintenance_rule_set_store::MaintenanceRuleSetStore, plugin_store::PluginStore,
    post_processing_script_store::PostProcessingScriptStore, rule_set_store::RuleSetStore,
};
pub(crate) use scryer_infrastructure_configuration::settings::{
    quality_profile_store::QualityProfileStore, settings_store::SettingsStore,
    subtitle_provider_config_store::SubtitleProviderConfigStore,
};
pub(crate) use scryer_infrastructure_datastore::{MigrationMode, SqliteServices, encryption};
#[cfg(test)]
pub(crate) use scryer_infrastructure_datastore::{
    postgres::PostgresServices, sqlite_url_with_create,
};
pub(crate) use scryer_infrastructure_identity::{
    oauth::store::OAuthStore,
    users::{store::UserStore, totp_store::TotpStore, webauthn_store::WebauthnStore},
};
#[cfg(feature = "image-processing")]
pub(crate) use scryer_infrastructure_library::media::images::processor::HttpTitleImageProcessor;
pub(crate) use scryer_infrastructure_library::media::{
    images::title_image_store::TitleImageStore,
    libraries::{
        location_operation_store::LocationOperationStore,
        scan_unmatched_store::LibraryScanUnmatchedStore,
        title_merge_store::TitleMergeStore,
        state_store::{
            BlocklistStore, HousekeepingStore, LibraryProbeStore, PendingReleaseStore,
            SubtitleDownloadStore, WantedStore,
        },
        store::LibraryStore,
    },
    requests::MediaRequestStore,
    search::media_file_store::MediaFileStore,
    servers::MediaServerConnectionStore,
    shows::store::ShowStore,
    signals::MediaServerSignalStore,
    titles::store::TitleStore,
};
pub(crate) use scryer_infrastructure_metadata::metadata::gateway::client::{
    MetadataGatewayClient, SmgEnrollmentConfig,
};
pub(crate) use scryer_infrastructure_notifications::notifications::store::NotificationStore;
#[cfg(test)]
pub(crate) use scryer_infrastructure_sql::types::SettingDefinitionSeed;
pub(crate) use scryer_infrastructure_workflow::workflow::{
    release_store::ReleaseStore,
    stores::{
        AcquisitionStore, DomainEventStore, DownloadQueueCommandStore, DownloadRegistryStore,
        DownloadSubmissionStore, ExternalImportMonitorStore, ExternalImportSetupSecretDraftStore,
        ImportStore, WorkflowOperationStore,
    },
};

pub use assembly::{
    DatastoreAssembly, DatastoreConfig, DatastoreConfigSource, DatastoreCustomizationStore,
    DatastoreEncryptionBootstrapReport, DatastoreEngine, datastore_file_path,
    resolve_datastore_config_from_env, restore_backup_bundle_to_datastore,
    restore_backup_bundle_to_datastore_path, restore_prepared_backup_directory_to_datastore,
    validate_datastore,
};
