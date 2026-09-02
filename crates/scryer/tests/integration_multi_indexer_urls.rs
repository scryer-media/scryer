//! End-to-end integration test that runs the full search pipeline
//! (discovery → multi-indexer → WASM plugins → HTTP) and captures
//! every outbound URL from real plugin binaries.
//!
//! Run with: cargo nextest run -E 'test(multi_indexer_url_trace)' --success-output immediate

mod common;

use std::sync::Arc;

use common::{
    disable_platform_keystore_for_tests, initialize_wasm_runtime_for_tests, load_fixture,
};
use scryer_application::{
    AppServices, AppUseCase, FacetRegistry, INDEXER_ROUTING_SETTINGS_KEY, IndexerPluginProvider,
    IndexerRoutingSettingsEntry, JwtAuthConfig, LibraryRootDraft, MovieFacetHandler,
    QUALITY_PROFILE_CATALOG_KEY, QUALITY_PROFILE_ID_KEY, SETTINGS_SCOPE_SYSTEM,
    SaveQualityProfileSettings, SeriesFacetHandler,
};
use scryer_domain::{ExternalId, IndexerConfig, MediaFacet, NewTitle, User};
use scryer_infrastructure_acquisition::{
    downloads::config_store::DownloadClientConfigStore,
    indexers::{
        config_store::IndexerConfigStore, search_client::MultiIndexerSearchClient,
        stats::InMemoryIndexerStatsTracker,
    },
};
use scryer_infrastructure_configuration::{
    customization::{
        plugin_store::PluginStore, post_processing_script_store::PostProcessingScriptStore,
        rule_set_store::RuleSetStore,
    },
    settings::{quality_profile_store::QualityProfileStore, settings_store::SettingsStore},
};
use scryer_infrastructure_datastore::SqliteServices;
use scryer_infrastructure_identity::users::store::UserStore;
use scryer_infrastructure_library::media::{
    images::title_image_store::TitleImageStore,
    libraries::{
        scan_unmatched_store::LibraryScanUnmatchedStore,
        scanner::FileSystemLibraryScanner,
        state_store::{
            HousekeepingStore, LibraryProbeStore, PendingReleaseStore, SubtitleDownloadStore,
            WantedStore,
        },
        store::LibraryStore,
    },
    search::media_file_store::MediaFileStore,
    shows::store::ShowStore,
    titles::store::TitleStore,
};
use scryer_infrastructure_sql::types::SettingDefinitionSeed;
use scryer_infrastructure_workflow::workflow::{
    release_store::ReleaseStore,
    stores::{
        AcquisitionStore, DomainEventStore, DownloadSubmissionStore, ImportStore,
        WorkflowOperationStore,
    },
};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const NEWZNAB_EMPTY: &str = r#"{"channel":{"item":[]}}"#;

/// Build a full AppUseCase with NZBGeek and Torznab plugins,
/// each backed by its own wiremock server. Creates indexer configs in SQLite
/// so the multi-indexer discovers them at search time.
async fn setup() -> (
    AppUseCase,
    User,
    MockServer, // newznab
    MockServer, // torznab
) {
    setup_with_indexer_configs(|newznab_server, torznab_server, now| {
        vec![
            newznab_indexer_config("newznab-1", "Newznab", newznab_server, now),
            indexer_config(
                "torznab-1",
                "Torznab",
                "torznab",
                format!("{}/api", torznab_server.uri()),
                now,
            ),
        ]
    })
    .await
}

async fn setup_with_indexer_configs<F>(
    build_indexer_configs: F,
) -> (
    AppUseCase,
    User,
    MockServer, // newznab
    MockServer, // torznab
)
where
    F: FnOnce(&MockServer, &MockServer, chrono::DateTime<chrono::Utc>) -> Vec<IndexerConfig>,
{
    disable_platform_keystore_for_tests();
    initialize_wasm_runtime_for_tests();

    let newznab_server = MockServer::start().await;
    let torznab_server = MockServer::start().await;

    // Mount catch-all empty responses
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(NEWZNAB_EMPTY))
        .mount(&newznab_server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(NEWZNAB_EMPTY))
        .mount(&torznab_server)
        .await;

    let db = SqliteServices::new(":memory:")
        .await
        .expect("in-memory SQLite");
    let datastore = db.datastore();
    let encryption_key_state = db.encryption_key_state();
    let indexer_config_store = Arc::new(IndexerConfigStore::new(
        datastore.clone(),
        encryption_key_state.clone(),
    ));
    let download_client_config_store = Arc::new(DownloadClientConfigStore::new(
        datastore.clone(),
        encryption_key_state.clone(),
    ));
    let release_store = Arc::new(ReleaseStore::new(
        datastore.clone(),
        encryption_key_state.clone(),
    ));
    let settings_store = Arc::new(SettingsStore::new(
        datastore.clone(),
        encryption_key_state.clone(),
    ));
    settings_store
        .batch_ensure_setting_definitions(vec![
            SettingDefinitionSeed {
                category: "media".into(),
                scope: SETTINGS_SCOPE_SYSTEM.into(),
                key_name: INDEXER_ROUTING_SETTINGS_KEY.into(),
                data_type: "string".into(),
                default_value_json: "{}".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "quality".into(),
                scope: SETTINGS_SCOPE_SYSTEM.into(),
                key_name: QUALITY_PROFILE_CATALOG_KEY.into(),
                data_type: "string".into(),
                default_value_json: "[]".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "quality".into(),
                scope: SETTINGS_SCOPE_SYSTEM.into(),
                key_name: QUALITY_PROFILE_ID_KEY.into(),
                data_type: "string".into(),
                default_value_json: "\"\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
        ])
        .await
        .expect("seed indexer routing setting definition");
    let quality_profile_store = Arc::new(QualityProfileStore::new(datastore.clone()));
    let domain_event_store = Arc::new(DomainEventStore::new(datastore.clone()));
    let acquisition_store = Arc::new(AcquisitionStore::new(datastore.clone()));
    let download_submission_store = Arc::new(DownloadSubmissionStore::new(datastore.clone()));
    let import_store = Arc::new(ImportStore::new(datastore.clone()));
    let workflow_operation_store = Arc::new(WorkflowOperationStore::new(datastore.clone()));

    // Load the remaining bundled indexer plugins.
    let plugin_provider: Arc<dyn IndexerPluginProvider> =
        Arc::new(scryer_plugins::DynamicPluginProvider::new(
            scryer_plugins::build_indexer_plugin_provider(&[], &[]),
        ));

    let indexer_stats: Arc<dyn scryer_application::IndexerStatsTracker> =
        Arc::new(InMemoryIndexerStatsTracker::new(None));

    let indexer_client = MultiIndexerSearchClient::new(
        indexer_config_store.clone(),
        indexer_stats.clone(),
        plugin_provider.clone(),
    );

    // Create indexer configs in SQLite so the multi-indexer finds them
    use scryer_application::IndexerConfigRepository;
    let now = chrono::Utc::now();
    for config in build_indexer_configs(&newznab_server, &torznab_server, now) {
        indexer_config_store
            .create(config)
            .await
            .expect("create indexer config");
    }

    scryer_application::DownloadClientConfigRepository::create(
        &*download_client_config_store,
        scryer_domain::DownloadClientConfig {
            id: "nzbget-1".into(),
            name: "NZBGet".into(),
            client_type: "nzbget".into(),
            config_json: serde_json::json!({
                "base_url": "http://localhost:1"
            })
            .to_string(),
            client_priority: 1,
            is_enabled: true,
            status: scryer_domain::DownloadClientStatus::default(),
            last_error: None,
            last_seen_at: None,
            created_at: now,
            updated_at: now,
            proxy_config_id: None,
        },
    )
    .await
    .expect("create download client config");

    // Build a minimal download client so AppServices doesn't panic
    let staged_nzb_dir = tempfile::TempDir::new().expect("failed to create staged nzb tempdir");
    let staged_nzb_store = Arc::new(
        scryer_infrastructure_acquisition::downloads::staged_nzb_store::FileSystemStagedNzbStore::new(staged_nzb_dir.path())
            .await
            .expect("staged nzb store"),
    );
    let nzbget = scryer_infrastructure_acquisition::downloads::clients::NzbgetDownloadClient::with_staged_nzb_store(
        "http://localhost:1".to_string(),
        None,
        None,
        "SCORE".to_string(),
        staged_nzb_store.clone(),
        Arc::new(tokio::sync::Semaphore::new(4)),
    );

    let smg = scryer_infrastructure_metadata::metadata::gateway::client::MetadataGatewayClient::new_with_enrollment_store(
        "http://localhost:2/graphql".to_string(),
        settings_store.clone(),
        scryer_infrastructure_metadata::metadata::gateway::client::SmgEnrollmentConfig {
            registration_secret: None,
        },
    );

    let title_store = Arc::new(TitleStore::new(datastore.clone()));
    let show_store = Arc::new(ShowStore::new(datastore.clone()));
    let user_store = Arc::new(UserStore::new(datastore.clone()));
    let library_store = Arc::new(LibraryStore::new(datastore.clone()));
    for (library_id, name, slug, root_path) in [
        ("movie_default_library", "Movies", "movies", "/data/movies"),
        ("series_default_library", "Series", "series", "/data/series"),
        ("anime_default_library", "Anime", "anime", "/data/anime"),
    ] {
        scryer_application::LibraryRepository::update(
            &*library_store,
            library_id,
            name.to_string(),
            slug.to_string(),
            vec![LibraryRootDraft {
                path: root_path.to_string(),
                is_default: true,
            }],
        )
        .await
        .expect("seed default library root");
    }
    let titles: Arc<dyn scryer_application::TitleRepository> = title_store;
    let shows: Arc<dyn scryer_application::ShowRepository> = show_store;
    let users: Arc<dyn scryer_application::UserRepository> = user_store;
    let libraries: Arc<dyn scryer_application::LibraryRepository> = library_store;
    let indexer_configs_repo: Arc<dyn scryer_application::IndexerConfigRepository> =
        indexer_config_store;
    let download_client_configs: Arc<dyn scryer_application::DownloadClientConfigRepository> =
        download_client_config_store;
    let release_attempts: Arc<dyn scryer_application::ReleaseAttemptRepository> = release_store;
    let settings: Arc<dyn scryer_application::SettingsRepository> = settings_store.clone();
    let quality_profiles: Arc<dyn scryer_application::QualityProfileRepository> =
        quality_profile_store.clone();

    let library_probe_store = Arc::new(LibraryProbeStore::new(datastore.clone()));
    let wanted_store = Arc::new(WantedStore::new(datastore.clone()));
    let pending_release_store = Arc::new(PendingReleaseStore::new(
        datastore.clone(),
        encryption_key_state.clone(),
    ));
    let blocklist_store = Arc::new(
        scryer_infrastructure_library::media::libraries::state_store::BlocklistStore::new(
            datastore.clone(),
        ),
    );
    let housekeeping_store = Arc::new(HousekeepingStore::new(datastore.clone()));
    let subtitle_download_store = Arc::new(SubtitleDownloadStore::new(datastore.clone()));
    let library_scan_unmatched_store = Arc::new(LibraryScanUnmatchedStore::new(datastore.clone()));
    let media_file_store = Arc::new(MediaFileStore::new(datastore.clone()));
    let title_image_store = Arc::new(TitleImageStore::new(datastore.clone()));
    let rule_set_store = Arc::new(RuleSetStore::new(datastore.clone()));
    let post_processing_script_store = Arc::new(PostProcessingScriptStore::new(datastore.clone()));
    let plugin_store = Arc::new(PluginStore::new(datastore.clone()));
    let services = AppServices::builder(
        titles,
        shows,
        users,
        indexer_configs_repo,
        Arc::new(indexer_client),
        Arc::new(nzbget),
        download_client_configs,
        release_attempts,
        settings,
        quality_profiles,
        ":memory:".to_string(),
    )
    .with_media_files(media_file_store)
    .with_libraries(libraries)
    .with_acquisition_scope_states(wanted_store)
    .with_pending_releases(pending_release_store)
    .with_blocklist_repo(blocklist_store)
    .with_library_probe_signatures(library_probe_store)
    .with_library_scan_unmatched_items(library_scan_unmatched_store)
    .with_title_images(title_image_store)
    .with_housekeeping(housekeeping_store)
    .with_subtitle_downloads(subtitle_download_store)
    .with_rule_set_store(rule_set_store)
    .with_post_processing_script_store(post_processing_script_store)
    .with_plugin_installation_store(plugin_store)
    .with_acquisition_state(acquisition_store)
    .with_domain_events(domain_event_store)
    .with_download_submissions(download_submission_store)
    .with_import_artifacts(import_store.clone())
    .with_imports(import_store)
    .with_job_runs(workflow_operation_store.clone())
    .with_system_info(settings_store)
    .with_metadata_gateway(Arc::new(smg))
    .with_library_scanner(Arc::new(FileSystemLibraryScanner::new()))
    .with_indexer_stats(indexer_stats)
    .with_plugin_provider(plugin_provider)
    .with_staged_nzb_store(staged_nzb_store)
    .with_workflow_operations(workflow_operation_store)
    .build();

    let mut registry = FacetRegistry::new();
    registry.register(Arc::new(MovieFacetHandler));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Series,
    )));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Anime,
    )));

    let app = AppUseCase::new(
        services,
        JwtAuthConfig {
            issuer: "scryer-test".into(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".into(),
        },
        Arc::new(registry),
    );

    // Create a test user with catalog and title permissions.
    let mut user = User {
        id: "test-user".into(),
        username: "tester".into(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };

    user.authorization = scryer_domain::UserAuthorization {
        app: scryer_domain::AppPermissionMask::from_permissions([
            scryer_domain::AppPermission::ManageCatalogSettings,
        ]),
        default_library: scryer_domain::LibraryPermissionMask::from_permissions([
            scryer_domain::LibraryPermission::View,
            scryer_domain::LibraryPermission::ManageTitles,
        ]),
        actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
        loaded: true,
        ..Default::default()
    };

    let profile = scryer_application::builtin_default_quality_profile();
    app.save_quality_profile_settings(
        &user,
        SaveQualityProfileSettings {
            global_profile_id: Some(profile.id.clone()),
            profiles: vec![profile],
            replace_existing: true,
            category_selections: Vec::new(),
            global_scoring_persona: None,
            category_persona_selections: Vec::new(),
        },
    )
    .await
    .expect("seed configured default quality profile");

    (app, user, newznab_server, torznab_server)
}

fn newznab_indexer_config(
    id: &str,
    name: &str,
    server: &MockServer,
    now: chrono::DateTime<chrono::Utc>,
) -> IndexerConfig {
    indexer_config(id, name, "newznab", format!("{}/api", server.uri()), now)
}

fn indexer_config(
    id: &str,
    name: &str,
    provider_type: &str,
    base_url: String,
    now: chrono::DateTime<chrono::Utc>,
) -> IndexerConfig {
    IndexerConfig {
        id: id.into(),
        name: name.into(),
        provider_type: provider_type.into(),
        base_url: base_url.clone(),
        api_key_encrypted: Some("test-api-key".into()),
        is_enabled: true,
        enable_interactive_search: true,
        enable_auto_search: true,
        proxy_config_id: None,
        download_client_id: None,
        seeding_profile_id: None,
        managed_parent_config_id: None,
        managed_child_key: None,
        managed_metadata_json: None,
        caps_snapshot_json: None,
        rate_limit_seconds: Some(0),
        rate_limit_burst: None,
        disabled_until: None,
        last_health_status: None,
        last_error_message: None,
        last_error_at: None,
        config_json: Some(
            serde_json::json!({
                "base_url": base_url,
                "api_key": "test-api-key",
            })
            .to_string(),
        ),
        created_at: now,
        updated_at: now,
    }
}

async fn add_search_title(
    app: &AppUseCase,
    user: &User,
    name: &str,
    facet: MediaFacet,
    external_ids: Vec<ExternalId>,
) -> String {
    let title = app
        .add_title(
            user,
            NewTitle {
                name: name.to_string(),
                facet,
                monitored: true,
                tags: vec![],
                external_ids,
                ..Default::default()
            },
        )
        .await
        .expect("add search title");

    title.id
}

fn external_id(source: &str, value: &str) -> ExternalId {
    ExternalId {
        source: source.to_string(),
        value: value.to_string(),
    }
}

async fn captured_urls(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .map(|r| r.url.to_string())
        .collect()
}

fn print_urls(label: &str, urls: &[String]) {
    if urls.is_empty() {
        println!("  {label}: (no calls)");
    } else {
        for url in urls {
            println!("  {label}: {url}");
        }
    }
}

fn print_summary(newznab: &[String], torznab: &[String]) {
    print_urls("Newznab", newznab);
    print_urls("Torznab", torznab);
    println!("  Total HTTP calls: {}", newznab.len() + torznab.len());
}

fn assert_id_only_then_fallback(urls: &[String], id_fragment: &str, fallback_query_fragment: &str) {
    assert!(
        !urls.is_empty(),
        "expected at least one request containing {id_fragment}"
    );
    assert!(
        urls[0].contains(id_fragment),
        "first request should use ID search: {:?}",
        urls
    );
    assert!(
        !urls[0].contains("&q="),
        "first request should not mix freetext into the ID tier: {:?}",
        urls[0]
    );
    assert!(
        urls.iter()
            .skip(1)
            .any(|url| url.contains(fallback_query_fragment)),
        "expected a later freetext fallback request containing {fallback_query_fragment}: {:?}",
        urls
    );
}

fn newznab_response_with_title(title: &str, guid: &str) -> String {
    serde_json::json!({
        "channel": {
            "title": "api.nzbgeek.info",
            "item": [{
                "title": title,
                "link": format!("https://api.nzbgeek.info/details/{guid}"),
                "pubDate": "Wed, 15 Jan 2025 12:00:00 +0000",
                "enclosure": {
                    "@attributes": {
                        "url": format!("https://api.nzbgeek.info/api?t=get&id={guid}&apikey=testkey"),
                        "length": "1073741824",
                        "type": "application/x-nzb"
                    }
                },
                "attr": [
                    { "@attributes": { "name": "size", "value": "1073741824" } },
                    { "@attributes": { "name": "guid", "value": guid } },
                    { "@attributes": { "name": "grabs", "value": "42" } },
                    { "@attributes": { "name": "password", "value": "1" } }
                ]
            }]
        }
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// Protected RAR routing — two Newznab configs sharing one endpoint
// ---------------------------------------------------------------------------

#[tokio::test]
async fn protected_rar_routing_flip_uses_fresh_enabled_shared_newznab_source() {
    let (app, user, newznab, _torznab) =
        setup_with_indexer_configs(|newznab_server, _torznab_server, now| {
            vec![
                newznab_indexer_config(
                    "newznab-nzb-password",
                    "E2E Protected RAR NZB Password",
                    newznab_server,
                    now,
                ),
                newznab_indexer_config(
                    "newznab-indexer-password",
                    "E2E Protected RAR Indexer Password",
                    newznab_server,
                    now,
                ),
            ]
        })
        .await;

    let nzb_title = "Paperman.2012.Protected.RAR.NZB.Password.1080p.WEB-DL-GROUP";
    let indexer_title = "Paperman.2012.Protected.RAR.Indexer.Password.1080p.WEB-DL-GROUP";
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("cat", "2000"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(newznab_response_with_title(
                nzb_title,
                "protected-rar-nzb-password",
            )),
        )
        .with_priority(1)
        .mount(&newznab)
        .await;
    Mock::given(method("GET"))
        .and(path("/api"))
        .and(query_param("cat", "2040"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(newznab_response_with_title(
                indexer_title,
                "protected-rar-indexer-password",
            )),
        )
        .with_priority(1)
        .mount(&newznab)
        .await;

    let title_id = add_search_title(
        &app,
        &user,
        "Paperman",
        MediaFacet::Movie,
        vec![external_id("imdb", "tt2388725")],
    )
    .await;

    app.update_indexer_routing(
        &user,
        "movie",
        vec![
            IndexerRoutingSettingsEntry {
                indexer_id: "newznab-nzb-password".to_string(),
                enabled: true,
                categories: vec!["2000".to_string()],
                priority: 1,
            },
            IndexerRoutingSettingsEntry {
                indexer_id: "newznab-indexer-password".to_string(),
                enabled: false,
                categories: vec!["2040".to_string()],
                priority: 2,
            },
        ],
    )
    .await
    .expect("write first routing state");

    let first_results = app
        .search_indexers_for_title(
            &user,
            title_id.clone(),
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("first search should succeed");
    let urls_after_first_search = captured_urls(&newznab).await;
    assert!(
        first_results.iter().any(|result| {
            result.title == nzb_title && result.source.contains("E2E Protected RAR NZB Password")
        }),
        "first search should attribute the NZB-password release to the enabled indexer: {first_results:?}; urls: {urls_after_first_search:?}"
    );
    assert!(
        first_results
            .iter()
            .all(|result| !result.source.contains("E2E Protected RAR Indexer Password")),
        "disabled indexer should not contribute first-search results: {first_results:?}"
    );
    app.update_indexer_routing(
        &user,
        "movie",
        vec![
            IndexerRoutingSettingsEntry {
                indexer_id: "newznab-nzb-password".to_string(),
                enabled: false,
                categories: vec!["2000".to_string()],
                priority: 1,
            },
            IndexerRoutingSettingsEntry {
                indexer_id: "newznab-indexer-password".to_string(),
                enabled: true,
                categories: vec!["2040".to_string()],
                priority: 2,
            },
        ],
    )
    .await
    .expect("write second routing state");

    let second_results = app
        .search_indexers_for_title(&user, title_id, tokio_util::sync::CancellationToken::new())
        .await
        .expect("second search should succeed");
    assert!(
        second_results.iter().any(|result| {
            result.title == indexer_title
                && result.source.contains("E2E Protected RAR Indexer Password")
        }),
        "second search should attribute the indexer-password release to the newly enabled indexer: {second_results:?}"
    );
    assert!(
        second_results
            .iter()
            .all(|result| !result.source.contains("E2E Protected RAR NZB Password")),
        "previously enabled indexer should not contribute second-search results: {second_results:?}"
    );

    let urls_after_second_search = captured_urls(&newznab).await;
    let second_search_urls = &urls_after_second_search[urls_after_first_search.len()..];
    assert!(
        second_search_urls
            .iter()
            .any(|url| url.contains("cat=2040")),
        "second search should query the newly enabled category: {second_search_urls:?}"
    );
    assert!(
        second_search_urls
            .iter()
            .all(|url| !url.contains("cat=2000")),
        "second search should not query the disabled category: {second_search_urls:?}"
    );
}

// ---------------------------------------------------------------------------
// Blade Summit S02E03 — anime episode, end-to-end through discovery layer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_indexer_url_trace_anime_episode() {
    let (app, user, newznab, torznab) = setup().await;
    let title_id = add_search_title(
        &app,
        &user,
        "Blade Summit",
        MediaFacet::Anime,
        vec![external_id("tvdb", "348545"), external_id("anidb", "1535")],
    )
    .await;

    let _results = app
        .search_indexers_for_episode(
            &user,
            title_id,
            "02".into(),
            "03".into(),
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("search should succeed");

    println!("\n=== Blade Summit S02E03 (anime, anidb=1535, tvdb=348545) ===");
    print_summary(
        &captured_urls(&newznab).await,
        &captured_urls(&torznab).await,
    );
}

// ---------------------------------------------------------------------------
// Cinder Line S05E01 — regular TV series
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_indexer_url_trace_series_episode() {
    let (app, user, newznab, torznab) = setup().await;
    let title_id = add_search_title(
        &app,
        &user,
        "Cinder Line",
        MediaFacet::Series,
        vec![external_id("tvdb", "81189")],
    )
    .await;

    let _results = app
        .search_indexers_for_episode(
            &user,
            title_id,
            "05".into(),
            "01".into(),
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("search should succeed");

    let newznab_urls = captured_urls(&newznab).await;
    let torznab_urls = captured_urls(&torznab).await;

    println!("\n=== Cinder Line S05E01 (series, tvdb=81189) ===");
    print_summary(&newznab_urls, &torznab_urls);

    assert_id_only_then_fallback(&newznab_urls, "tvdbid=81189", "q=Cinder+Line");
    assert_id_only_then_fallback(&torznab_urls, "tvdbid=81189", "q=Cinder+Line");
}

// ---------------------------------------------------------------------------
// Lattice Zero — movie with imdb_id only
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_indexer_url_trace_movie() {
    let (app, user, newznab, torznab) = setup().await;
    let title_id = add_search_title(
        &app,
        &user,
        "Lattice Zero",
        MediaFacet::Movie,
        vec![external_id("imdb", "tt0133093")],
    )
    .await;

    let _results = app
        .search_indexers_for_title(&user, title_id, tokio_util::sync::CancellationToken::new())
        .await
        .expect("search should succeed");

    let newznab_urls = captured_urls(&newznab).await;
    let torznab_urls = captured_urls(&torznab).await;

    println!("\n=== Lattice Zero (movie, imdb=tt0133093) ===");
    print_summary(&newznab_urls, &torznab_urls);

    assert_id_only_then_fallback(&newznab_urls, "imdbid=000133093", "q=Lattice+Zero");
    assert_id_only_then_fallback(&torznab_urls, "imdbid=000133093", "q=Lattice+Zero");
}

// ---------------------------------------------------------------------------
// Lantern Tide — movie with imdb_id + anidb_id (from metadata hydration)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multi_indexer_url_trace_movie_lantern_tide() {
    let (app, user, newznab, torznab) = setup().await;
    let title_id = add_search_title(
        &app,
        &user,
        "Lantern Tide: Hidden Current",
        MediaFacet::Movie,
        vec![
            external_id("imdb", "tt0245429"),
            external_id("anidb", "112"),
        ],
    )
    .await;

    let newznab_fixture = load_fixture("nzbgeek/search_movie.json").replace(
        "Movie.Title.2024.2160p.UHD.BluRay.REMUX.DV.HDR.DTS-HD.MA.7.1.HEVC-GROUP",
        "Lantern.Tide.Hidden.Current.2001.1080p.BluRay",
    );
    Mock::given(method("GET"))
        .and(path("/api/api"))
        .and(query_param("imdbid", "000245429"))
        .respond_with(ResponseTemplate::new(200).set_body_string(newznab_fixture))
        .with_priority(1)
        .mount(&newznab)
        .await;

    let _results = app
        .search_indexers_for_title(&user, title_id, tokio_util::sync::CancellationToken::new())
        .await
        .expect("search should succeed");

    let newznab_urls = captured_urls(&newznab).await;
    let torznab_urls = captured_urls(&torznab).await;

    println!("\n=== Lantern Tide (movie, imdb=tt0245429, anidb=112) ===");
    print_summary(&newznab_urls, &torznab_urls);
    assert_id_only_then_fallback(
        &newznab_urls,
        "imdbid=000245429",
        "q=Lantern+Tide+Hidden+Current",
    );
    assert_id_only_then_fallback(
        &torznab_urls,
        "imdbid=000245429",
        "q=Lantern+Tide+Hidden+Current",
    );
}
