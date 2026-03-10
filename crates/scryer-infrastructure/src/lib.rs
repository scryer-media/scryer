pub(crate) mod commands;
mod download_clients;
pub mod external_import;
mod file_importer;
mod indexer_stats;
mod library_scanner;
mod library_renamer;
mod metadata_gateway;
mod migrations;
mod queries;
mod repositories;
mod sqlite_services;
mod types;
pub mod encryption;
pub mod jwt_keys;
pub mod smg_enrollment;


#[cfg(test)]
mod tests;

pub use file_importer::FsFileImporter;
pub use indexer_stats::InMemoryIndexerStatsTracker;
pub use download_clients::{
    MultiIndexerSearchClient, NzbgetDownloadClient, SabnzbdDownloadClient,
    PrioritizedDownloadClientRouter, WeaverDownloadClient,
    start_weaver_subscription_bridge,
};
pub use library_renamer::FileSystemLibraryRenamer;
pub use library_scanner::FileSystemLibraryScanner;
pub use metadata_gateway::{MetadataGatewayClient, SmgEnrollmentConfig};
pub use migrations::{list_embedded_migration_keys, list_embedded_migrations};
pub use encryption::EncryptionKey;
pub use sqlite_services::SqliteServices;
pub(crate) use types::sqlite_url_with_create;
pub use types::{
    EmbeddedMigrationDescriptor, MigrationMode, MigrationStatus, SettingDefinitionSeed,
    SettingsDefinitionRecord, SettingsValueRecord, WorkflowOperationRecord,
};
