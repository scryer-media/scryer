use super::*;
use chrono::Utc;
use scryer_application::{
    AcquisitionScopeState, AcquisitionScopeStateRepository, AcquisitionScopeStatesQuery,
    AcquisitionScopeStatus, AppError, AppResult, ClientJobLocator, CollectionUpdate,
    DomainEventRepository, DownloadClientConfigRepository, DownloadQueueCommandRepository,
    DownloadSubmission, DownloadSubmissionIdentity, DownloadSubmissionRepository, EpisodeUpdate,
    HousekeepingRepository, ImportRepository, InsertMediaFileInput, LibraryScanUnmatchedItem,
    LibraryScanUnmatchedItemRepository, LibraryScanUnmatchedSearchAttempt, MediaFileRepository,
    MediaFileRole, MetadataFieldUpdate, NotificationChannelRepository,
    NotificationSubscriptionRepository, OAuthRepository, PendingImportStatus,
    PendingReleaseRepository, PluginInstallationRepository, ReleaseAttemptRepository,
    ReleaseDecision, ReleaseDownloadAttemptOutcome, ScopedExternalId, SettingsRepository,
    ShowRepository, SortDirection, SubmissionScope, SubtitleDownloadRepository,
    SubtitleProviderConfigRepository, SubtitleProviderConfigUpdate, TitleArtworkUrlUpdate,
    TitleCatalogFilter, TitleCatalogSort, TitleCatalogSortKey, TitleCredit, TitleExternalIdLookup,
    TitleExternalRating, TitleImageBlob, TitleImageKind, TitleImageRepository,
    TitleImageSourceResult, TitleImageVariantRecord, TitleMetadataUpdate, TitleRatingSummary,
    TitleRepository, UserRepository,
    subtitles::{ExternalSubtitleDetectionSource, ExternalSubtitleProbeCacheEntry},
};
use scryer_domain::{
    ChannelType, Collection, CollectionType, DomainEventFilter, DomainEventPayload,
    DomainEventStream, DownloadClientConfig, DownloadClientStatus, Episode, ExternalId, Id,
    ImportStatus, ImportType, MediaFacet, NewDomainEvent, NotificationChannelConfig,
    NotificationEventType, NotificationSubscription, SubtitleProviderConfig, TaggedAlias, Title,
    TitleContextSnapshot, TitleUpdatedEventData,
};
use sqlx::{Row, sqlite::SqlitePoolOptions};
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use tokio::time::{Duration, timeout};

mod canonical_download_binding_staleness;
mod canonical_download_registry;
mod discovery_pending_context_changes;
mod emby_media_servers;
mod external_import_setup_secret_drafts;
mod imports_download_submissions;
mod library_scan_unmatched;
mod maintenance_rule_sets;
mod migrations;
mod oauth;
mod permissions_users_shows;
mod plugins;
mod scope_indexer_coverage;
mod settings_and_writer;
mod sql_runtime_gated_write;
mod stores_migrations_regressions;
mod title_images;
mod titles_metadata;
mod wanted_items_and_search;
mod workflow_operation;

fn test_descriptor_json(
    plugin_id: &str,
    version: &str,
    plugin_type: &str,
    provider_type: &str,
) -> String {
    fn indexer_config_fields() -> Vec<scryer_plugin_sdk::ConfigFieldDef> {
        vec![scryer_plugin_sdk::ConfigFieldDef {
            key: "base_url".to_string(),
            label: "Base URL".to_string(),
            field_type: scryer_plugin_sdk::ConfigFieldType::String,
            required: true,
            default_value: None,
            value_source: scryer_plugin_sdk::ConfigFieldValueSource::User,
            role: Some(scryer_plugin_sdk::ConfigFieldRole::ConnectionUrl),
            host_binding: None,
            options: Vec::new(),
            help_text: None,
        }]
    }

    let provider = match plugin_type {
        "indexer" => {
            scryer_plugin_sdk::ProviderDescriptor::Indexer(scryer_plugin_sdk::IndexerDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: Vec::new(),
                provider_profiles: Vec::new(),
                source_kind: scryer_plugin_sdk::IndexerSourceKind::Generic,
                capabilities: Default::default(),
                scoring_policies: Vec::new(),
                config_fields: indexer_config_fields(),
                allowed_hosts: Vec::new(),
                rate_limit_seconds: None,
                search_semantics_version: None,
                strategy_plan: None,
            })
        }
        "usenet_indexer" => {
            scryer_plugin_sdk::ProviderDescriptor::Indexer(scryer_plugin_sdk::IndexerDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: Vec::new(),
                provider_profiles: Vec::new(),
                source_kind: scryer_plugin_sdk::IndexerSourceKind::Usenet,
                capabilities: Default::default(),
                scoring_policies: Vec::new(),
                config_fields: indexer_config_fields(),
                allowed_hosts: Vec::new(),
                rate_limit_seconds: None,
                search_semantics_version: None,
                strategy_plan: None,
            })
        }
        "torrent_indexer" => {
            scryer_plugin_sdk::ProviderDescriptor::Indexer(scryer_plugin_sdk::IndexerDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: Vec::new(),
                provider_profiles: Vec::new(),
                source_kind: scryer_plugin_sdk::IndexerSourceKind::Torrent,
                capabilities: Default::default(),
                scoring_policies: Vec::new(),
                config_fields: indexer_config_fields(),
                allowed_hosts: Vec::new(),
                rate_limit_seconds: None,
                search_semantics_version: None,
                strategy_plan: None,
            })
        }
        "notification" => scryer_plugin_sdk::ProviderDescriptor::Notification(
            scryer_plugin_sdk::NotificationDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: Vec::new(),
                config_fields: Vec::new(),
                default_base_url: None,
                allowed_hosts: Vec::new(),
                capabilities: Default::default(),
            },
        ),
        "download_client" => scryer_plugin_sdk::ProviderDescriptor::DownloadClient(
            scryer_plugin_sdk::DownloadClientDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: Vec::new(),
                config_fields: Vec::new(),
                default_base_url: None,
                allowed_hosts: Vec::new(),
                accepted_inputs: Vec::new(),
                isolation_modes: Vec::new(),
                capabilities: Default::default(),
            },
        ),
        "subtitle_provider" => {
            scryer_plugin_sdk::ProviderDescriptor::Subtitle(scryer_plugin_sdk::SubtitleDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: Vec::new(),
                config_fields: Vec::new(),
                default_base_url: None,
                allowed_hosts: Vec::new(),
                capabilities: Default::default(),
            })
        }
        other => panic!("unsupported test plugin type: {other}"),
    };

    serde_json::to_string(&scryer_plugin_sdk::PluginDescriptor {
        id: plugin_id.to_string(),
        name: format!("{plugin_id} Plugin"),
        version: version.to_string(),
        sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
        sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
        socket_permissions: Vec::new(),
        provider,
    })
    .expect("serialize test descriptor")
}

async fn import_store_test_harness(max_connections: u32) -> (sqlx::SqlitePool, ImportStore) {
    let pool = SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect("sqlite::memory:")
        .await
        .expect("pool should initialize");
    sqlx::query(
        "CREATE TABLE imports (
            id TEXT PRIMARY KEY,
            source_client_id TEXT,
            source_system TEXT NOT NULL,
            source_ref TEXT NOT NULL,
            import_type TEXT NOT NULL,
            status TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            result_json TEXT,
            rename_plan_json TEXT,
            download_id TEXT,
            canonical_download_id TEXT,
            started_at TEXT,
            finished_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            import_transfer_phase TEXT,
            import_transfer_bytes INTEGER,
            import_transfer_total_bytes INTEGER,
            import_transfer_started_at TEXT,
            import_transfer_updated_at TEXT
        )",
    )
    .execute(&pool)
    .await
    .expect("imports table should create");
    sqlx::query(
        "CREATE UNIQUE INDEX idx_imports_source_ref
         ON imports (COALESCE(source_client_id, ''), source_system, source_ref, import_type)
         WHERE download_id IS NULL",
    )
    .execute(&pool)
    .await
    .expect("imports identity index should create");
    sqlx::query(
        "CREATE UNIQUE INDEX idx_imports_active_download_id
         ON imports (COALESCE(source_client_id, ''), source_system, download_id)
         WHERE download_id IS NOT NULL
           AND status IN ('pending', 'running', 'processing')",
    )
    .execute(&pool)
    .await
    .expect("active download identity index should create");

    let workflow = ImportStore::new(crate::queries::sql_runtime::StoreDatastore::Sqlite {
        pool: pool.clone(),
        writer_gate: Arc::new(tokio::sync::Mutex::new(())),
    });

    (pool, workflow)
}

#[expect(
    dead_code,
    reason = "retained cross-datastore canonical identity fixture"
)]
fn orphan_test_submission(item_id: &str, source_title: &str) -> DownloadSubmission {
    DownloadSubmission {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        title_id: String::new(),
        purpose: scryer_application::DownloadSubmissionPurpose::Standard,
        facet: String::new(),
        download_client_id: Some("client-a".to_string()),
        download_client_type: "qbittorrent".to_string(),
        download_client_item_id: item_id.to_string(),
        source_hint: None,
        source_provider_id: None,
        source_provider_name: None,
        source_kind: None,
        source_title: Some(source_title.to_string()),
        info_hash: None,
        release_size_bytes: None,
        request_signature: None,
        scope: SubmissionScope::Orphan,
    }
}

#[expect(
    dead_code,
    reason = "retained cross-datastore canonical identity fixture"
)]
fn managed_episode_set_test_submission(item_id: &str) -> DownloadSubmission {
    DownloadSubmission {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        title_id: "title-managed".to_string(),
        purpose: scryer_application::DownloadSubmissionPurpose::AdditionalFile,
        facet: "anime".to_string(),
        download_client_id: Some("client-a".to_string()),
        download_client_type: "qbittorrent".to_string(),
        download_client_item_id: item_id.to_string(),
        source_hint: Some("magnet:?xt=urn:btih:feedface".to_string()),
        source_provider_id: None,
        source_provider_name: None,
        source_kind: Some(scryer_application::DownloadSourceKind::TorrentFile),
        source_title: Some("Managed.Release.S01".to_string()),
        info_hash: None,
        release_size_bytes: None,
        request_signature: Some("request-signature-1".to_string()),
        scope: SubmissionScope::EpisodeSet {
            episode_ids: vec!["episode-1".to_string(), "episode-2".to_string()],
        },
    }
}

#[expect(
    dead_code,
    reason = "retained cross-datastore canonical identity fixture"
)]
async fn assert_download_submission_orphan_precedence(
    workflow: &DownloadSubmissionStore,
) -> AppResult<()> {
    let item_id = "feedfacefeedfacefeedfacefeedfacefeedface";
    let source_identity = ClientJobLocator::new(Some("client-a"), "qbittorrent", item_id);

    workflow
        .record_submission(orphan_test_submission(item_id, "Foreign.Observation"))
        .await?;
    let orphan = workflow
        .find_by_client_item_id(&source_identity)
        .await?
        .expect("orphan row should insert");
    assert!(orphan.title_id.is_empty());
    assert!(matches!(orphan.scope, SubmissionScope::Orphan));

    workflow
        .record_submission_with_identity(
            managed_episode_set_test_submission(item_id),
            DownloadSubmissionIdentity {
                download_id: Some("download-feedface".to_string()),
            },
            None,
        )
        .await?;

    let managed = workflow
        .find_by_client_item_id(&source_identity)
        .await?
        .expect("managed row should replace orphan");
    assert_eq!(managed.title_id, "title-managed");
    assert_eq!(managed.facet, "anime");
    assert_eq!(managed.source_title.as_deref(), Some("Managed.Release.S01"));
    assert_eq!(
        managed.source_kind.map(|kind| kind.as_str()),
        Some("torrent_file")
    );
    assert_eq!(
        managed.request_signature.as_deref(),
        Some("request-signature-1")
    );
    assert_eq!(managed.purpose.as_str(), "additional_file");
    assert_eq!(
        managed.scope.episode_ids().unwrap_or(&[]),
        &["episode-1".to_string(), "episode-2".to_string()]
    );

    let submission_identity = workflow
        .get_submission_identity(&source_identity)
        .await?
        .expect("managed row should keep accepted identity");
    assert_eq!(
        submission_identity.download_id.as_deref(),
        Some("download-feedface")
    );
    let by_download_id = workflow
        .list_by_download_id(Some("client-a"), "qbittorrent", "download-feedface")
        .await?;
    assert_eq!(by_download_id.len(), 1);
    assert_eq!(by_download_id[0].title_id, "title-managed");

    workflow
        .record_submission(orphan_test_submission(item_id, "Late.Foreign.Observation"))
        .await?;

    let still_managed = workflow
        .find_by_client_item_id(&source_identity)
        .await?
        .expect("managed row should survive late orphan");
    assert_eq!(still_managed.title_id, "title-managed");
    assert_eq!(still_managed.facet, "anime");
    assert_eq!(
        still_managed.source_title.as_deref(),
        Some("Managed.Release.S01")
    );
    assert_eq!(
        still_managed.scope.episode_ids().unwrap_or(&[]),
        &["episode-1".to_string(), "episode-2".to_string()]
    );
    Ok(())
}

fn make_test_title(id: &str, poster_url: Option<&str>) -> Title {
    Title {
        id: id.to_string(),
        name: "Poster Test".to_string(),
        facet: MediaFacet::Movie,
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        monitored: true,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: vec![],
        root_folder_id: scryer_domain::root_folder_id_for_path("/data/movies"),
        created_by: None,
        created_at: Utc::now(),
        year: Some(2026),
        overview: Some("overview".to_string()),
        poster_url: poster_url.map(str::to_string),
        poster_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        catalog_sort_key: String::new(),
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        popularity: None,
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: vec![],
        tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    }
}

fn title_store(services: &SqliteServices) -> TitleStore {
    TitleStore::new(services.datastore())
}

fn show_store(services: &SqliteServices) -> ShowStore {
    ShowStore::new(services.datastore())
}

fn user_store(services: &SqliteServices) -> UserStore {
    UserStore::new(services.datastore())
}

fn oauth_store(services: &SqliteServices) -> OAuthStore {
    OAuthStore::new(services.datastore())
}

fn wanted_store(services: &SqliteServices) -> WantedStore {
    WantedStore::new(services.datastore())
}

fn housekeeping_store(services: &SqliteServices) -> HousekeepingStore {
    HousekeepingStore::new(services.datastore())
}

fn subtitle_download_store(services: &SqliteServices) -> SubtitleDownloadStore {
    SubtitleDownloadStore::new(services.datastore())
}

fn media_file_store(services: &SqliteServices) -> MediaFileStore {
    MediaFileStore::new(services.datastore())
}

fn library_scan_unmatched_store(services: &SqliteServices) -> LibraryScanUnmatchedStore {
    LibraryScanUnmatchedStore::new(services.datastore())
}

fn title_image_store(services: &SqliteServices) -> TitleImageStore {
    TitleImageStore::new(services.datastore())
}

fn discovery_store(services: &SqliteServices) -> crate::discovery::store::DiscoveryStore {
    crate::discovery::store::DiscoveryStore::new(services.datastore())
}

async fn temp_services(prefix: &str) -> (SqliteServices, std::path::PathBuf) {
    let db = std::env::temp_dir().join(format!(
        "{}_{}.db",
        prefix,
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    (services, db)
}

async fn run_embedded_migration(pool: &sqlx::SqlitePool, sql: &str) {
    for statement in sql
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
    {
        sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()))
            .execute(pool)
            .await
            .expect("migration statement should succeed");
    }
}

fn rolled_up_migration_section<'a>(rollup: &'a str, original_file: &str) -> &'a str {
    let marker = format!("-- Rolled up from {original_file}\n");
    let start = rollup
        .find(&marker)
        .unwrap_or_else(|| panic!("missing rollup section for {original_file}"))
        + marker.len();
    let rest = &rollup[start..];
    let end = rest.find("\n-- Rolled up from ").unwrap_or(rest.len());
    &rest[..end]
}

async fn single_connection_services(name: &str) -> (SqliteServices, std::path::PathBuf) {
    crate::spellfix::register_spellfix_auto_extension()
        .expect("spellfix auto-extension should register before migrations");

    let db = std::env::temp_dir().join(format!(
        "{}_{}.db",
        name,
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("single-connection pool should open");

    crate::migrations::run_migrations(&pool, crate::types::MigrationMode::Apply)
        .await
        .expect("migrations should apply");

    let services = SqliteServices {
        pool,
        encryption_key: Arc::new(RwLock::new(None)),
        writer_gate: Arc::new(tokio::sync::Mutex::new(())),
    };

    (services, db)
}

async fn create_pre_0079_title_projection_schema(pool: &sqlx::SqlitePool) {
    sqlx::query(
        "CREATE TABLE titles (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            facet TEXT NOT NULL,
            external_ids TEXT NOT NULL DEFAULT '[]',
            metadata_fetched_at TEXT
        )",
    )
    .execute(pool)
    .await
    .expect("create legacy titles");

    sqlx::query(
        "CREATE TABLE title_external_ids (
            id TEXT PRIMARY KEY,
            title_id TEXT NOT NULL,
            source TEXT NOT NULL,
            external_id TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .expect("create legacy title_external_ids");

    sqlx::query(
        "CREATE UNIQUE INDEX idx_title_external_ids_lookup
         ON title_external_ids(source, external_id)",
    )
    .execute(pool)
    .await
    .expect("create legacy title_external_ids lookup");
}

fn test_title_image_source_result(
    kind: TitleImageKind,
    source_url: &str,
    variant_key: &str,
    width: i32,
    height: i32,
    digest: &str,
) -> TitleImageSourceResult {
    test_title_image_source_result_with_variants(
        kind,
        source_url,
        vec![test_title_image_variant_record(
            variant_key,
            width,
            height,
            digest,
        )],
    )
}

fn test_title_image_source_result_with_variants(
    kind: TitleImageKind,
    source_url: &str,
    variants: Vec<TitleImageVariantRecord>,
) -> TitleImageSourceResult {
    TitleImageSourceResult {
        kind,
        requested_source_url: source_url.to_string(),
        source_url: source_url.to_string(),
        source_etag: None,
        source_last_modified: None,
        source_format: "jpeg".to_string(),
        source_width: 1000,
        source_height: 1500,
        variants,
    }
}

fn test_title_image_variant_record(
    variant_key: &str,
    width: i32,
    height: i32,
    seed: &str,
) -> TitleImageVariantRecord {
    let bytes = seed.as_bytes().to_vec();
    TitleImageVariantRecord {
        variant_key: variant_key.to_string(),
        format: "avif".to_string(),
        width,
        height,
        digest: format!("blake3:{}", blake3::hash(&bytes).to_hex()),
        bytes,
    }
}

fn test_title_image_version(seed: &str) -> String {
    blake3::hash(seed.as_bytes())
        .to_hex()
        .chars()
        .take(16)
        .collect()
}

fn assert_variant_target(
    task: &scryer_application::TitleImageSyncTask,
    kind: TitleImageKind,
    variant_key: &str,
) {
    assert_eq!(task.kind, kind);
    assert!(
        task.variants
            .iter()
            .any(|variant| variant.variant_key == variant_key)
    );
}
