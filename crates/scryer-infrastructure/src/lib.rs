pub(crate) mod commands;
mod download_clients;
pub mod encryption;
pub mod external_import;
mod file_importer;
mod indexer_stats;
pub mod keystore;
mod library_renamer;
mod library_scanner;
mod metadata_gateway;
mod migrations;
pub mod queries;
mod repositories;
pub mod smg_enrollment;
mod sqlite_services;
mod title_images;
mod types;

#[cfg(test)]
mod tests;

pub use download_clients::{
    MultiIndexerSearchClient, NzbgetDownloadClient, PrioritizedDownloadClientRouter,
    SabnzbdDownloadClient, WeaverDownloadClient, start_weaver_subscription_bridge,
};
pub use encryption::EncryptionKey;
pub use file_importer::FsFileImporter;
pub use indexer_stats::InMemoryIndexerStatsTracker;
pub use library_renamer::FileSystemLibraryRenamer;
pub use library_scanner::FileSystemLibraryScanner;
pub use metadata_gateway::{MetadataGatewayClient, SmgEnrollmentConfig};
pub use migrations::{list_embedded_migration_keys, list_embedded_migrations};
pub use sqlite_services::SqliteServices;
pub use title_images::SqliteTitleImageProcessor;
pub(crate) use types::sqlite_url_with_create;
pub use types::{
    EmbeddedMigrationDescriptor, MigrationMode, MigrationStatus, SettingDefinitionSeed,
    SettingsDefinitionRecord, SettingsValueRecord, WorkflowOperationRecord,
};
