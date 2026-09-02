//! Title-less indexer-search job integration tests (spec 0002, WP1).
//!
//! Drives the app-layer job methods against the full production-parity search
//! pipeline — `MultiIndexerSearchClient` with the REAL `InMemoryUpstreamScheduler`
//! → real WASM newznab plugin → wiremock. The bootstrap mirrors
//! `integration_interactive_release_search.rs`.

#![recursion_limit = "256"]

mod common;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{disable_platform_keystore_for_tests, initialize_wasm_runtime_for_tests};
use scryer_application::{
    AppServices, AppUseCase, FacetRegistry, INDEXER_ROUTING_SETTINGS_KEY, IndexerPluginProvider,
    IndexerSearchIndexerStatus, IndexerSearchKind, IndexerSearchRequest, IndexerSearchSnapshot,
    IndexerSearchState, JwtAuthConfig, LibraryRootDraft, MovieFacetHandler,
    QUALITY_PROFILE_CATALOG_KEY, QUALITY_PROFILE_ID_KEY, SETTINGS_SCOPE_SYSTEM,
    SaveQualityProfileSettings, SeriesFacetHandler,
};
use scryer_domain::{IndexerConfig, User};
use scryer_infrastructure_acquisition::{
    downloads::config_store::DownloadClientConfigStore,
    indexers::{
        config_store::IndexerConfigStore, search_client::MultiIndexerSearchClient,
        stats::InMemoryIndexerStatsTracker,
    },
    upstream_scheduler::InMemoryUpstreamScheduler,
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
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn newznab_response_with_title(title: &str, guid: &str) -> String {
    serde_json::json!({
        "channel": {
            "title": "probe.indexer",
            "item": [{
                "title": title,
                "link": format!("https://probe.indexer/details/{guid}"),
                "pubDate": "Wed, 15 Jan 2025 12:00:00 +0000",
                "enclosure": {
                    "@attributes": {
                        "url": format!("https://probe.indexer/api?t=get&id={guid}&apikey=testkey"),
                        "length": "1073741824",
                        "type": "application/x-nzb"
                    }
                },
                "attr": [
                    { "@attributes": { "name": "size", "value": "1073741824" } },
                    { "@attributes": { "name": "guid", "value": guid } },
                    { "@attributes": { "name": "grabs", "value": "42" } }
                ]
            }]
        }
    })
    .to_string()
}

fn indexer_config(
    id: &str,
    base_url: String,
    api_key: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> IndexerConfig {
    IndexerConfig {
        id: id.into(),
        name: id.into(),
        provider_type: "newznab".into(),
        base_url: base_url.clone(),
        api_key_encrypted: Some(api_key.into()),
        is_enabled: true,
        enable_interactive_search: true,
        enable_auto_search: true,
        indexer_proxy_config_id: None,
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
                "api_key": api_key,
            })
            .to_string(),
        ),
        created_at: now,
        updated_at: now,
    }
}

async fn setup_app(configs: Vec<IndexerConfig>) -> (AppUseCase, User) {
    disable_platform_keystore_for_tests();
    initialize_wasm_runtime_for_tests();

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

    let plugin_provider: Arc<dyn IndexerPluginProvider> =
        Arc::new(scryer_plugins::DynamicPluginProvider::new(
            scryer_plugins::build_indexer_plugin_provider(&[], &[]),
        ));

    let indexer_stats: Arc<dyn scryer_application::IndexerStatsTracker> =
        Arc::new(InMemoryIndexerStatsTracker::new(None));

    // Production parity: the REAL scheduler (TestContext uses the Null one).
    let indexer_client = MultiIndexerSearchClient::new(
        indexer_config_store.clone(),
        indexer_stats.clone(),
        plugin_provider.clone(),
    )
    .with_upstream_scheduler(Arc::new(InMemoryUpstreamScheduler::new()));

    use scryer_application::IndexerConfigRepository;
    let now = chrono::Utc::now();
    for config in configs {
        indexer_config_store
            .create(config)
            .await
            .expect("create indexer config");
    }

    // A live (empty) NZBGet mock keeps download-client lookups from adding
    // connect timeouts to the job.
    let nzbget_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"version":"1.1","result":[]}"#),
        )
        .mount(&nzbget_server)
        .await;
    let nzbget_url = nzbget_server.uri();
    std::mem::forget(nzbget_server);

    scryer_application::DownloadClientConfigRepository::create(
        &*download_client_config_store,
        scryer_domain::DownloadClientConfig {
            id: "nzbget-1".into(),
            name: "NZBGet".into(),
            client_type: "nzbget".into(),
            config_json: serde_json::json!({ "base_url": nzbget_url }).to_string(),
            client_priority: 1,
            is_enabled: true,
            status: scryer_domain::DownloadClientStatus::default(),
            last_error: None,
            last_seen_at: None,
            created_at: now,
            updated_at: now,
        },
    )
    .await
    .expect("create download client config");

    let staged_nzb_dir = tempfile::TempDir::new().expect("failed to create staged nzb tempdir");
    let staged_nzb_store = Arc::new(
        scryer_infrastructure_acquisition::downloads::staged_nzb_store::FileSystemStagedNzbStore::new(staged_nzb_dir.path())
            .await
            .expect("staged nzb store"),
    );
    std::mem::forget(staged_nzb_dir);
    let nzbget = scryer_infrastructure_acquisition::downloads::clients::NzbgetDownloadClient::with_staged_nzb_store(
        nzbget_url,
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

    let mut user = User {
        id: "test-user".into(),
        username: "tester".into(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    user.authorization = scryer_domain::UserAuthorization {
        // The Indexers page's gate (D13) plus the catalog gate the quality
        // profile seeding below needs.
        app: scryer_domain::AppPermissionMask::from_permissions([
            scryer_domain::AppPermission::ManageCatalogSettings,
            scryer_domain::AppPermission::ManageSystemSettings,
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

    (app, user)
}

async fn mount_healthy(server: &MockServer, title: &str, guid: &str) {
    Mock::given(method("GET"))
        .and(path("/api"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(newznab_response_with_title(title, guid)),
        )
        .mount(server)
        .await;
}

async fn mount_broken(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api"))
        .respond_with(ResponseTemplate::new(500).set_body_string("upstream exploded"))
        .mount(server)
        .await;
}

fn indexer_view<'a>(
    snapshot: &'a IndexerSearchSnapshot,
    indexer_id: &str,
) -> &'a scryer_application::IndexerSearchIndexerView {
    snapshot
        .indexers
        .iter()
        .find(|view| view.indexer_id == indexer_id)
        .unwrap_or_else(|| panic!("indexer {indexer_id} missing from snapshot: {snapshot:?}"))
}

async fn wait_for_terminal(
    app: &AppUseCase,
    user: &User,
    job_id: &str,
    deadline: Duration,
) -> IndexerSearchSnapshot {
    let started = Instant::now();
    loop {
        let snapshot = app
            .indexer_search(user, job_id)
            .await
            .expect("poll indexer search")
            .expect("job should be present in registry");
        if snapshot.state != IndexerSearchState::Running || started.elapsed() > deadline {
            return snapshot;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn raw_request(query: &str) -> IndexerSearchRequest {
    IndexerSearchRequest {
        query: query.to_string(),
        kind: IndexerSearchKind::Raw,
        ..IndexerSearchRequest::default()
    }
}

/// One broken indexer must not blank the batch, and Retry failed must heal it
/// without duplicating the healthy indexer's rows.
#[tokio::test]
async fn broken_indexer_fails_visibly_and_retry_heals_it_without_duplicates() {
    let healthy = MockServer::start().await;
    mount_healthy(&healthy, "Paperman.2012.1080p.WEB-DL-GRP", "ok-1").await;
    let broken = MockServer::start().await;
    mount_broken(&broken).await;

    let now = chrono::Utc::now();
    let (app, user) = setup_app(vec![
        indexer_config("healthy-a", format!("{}/api", healthy.uri()), "key-a", now),
        indexer_config("broken-b", format!("{}/api", broken.uri()), "key-b", now),
    ])
    .await;

    let start = app
        .start_indexer_search(&user, raw_request("Paperman"))
        .await
        .expect("start indexer search");
    assert_eq!(start.state, IndexerSearchState::Running);
    assert_eq!(start.indexers.len(), 2, "{start:?}");

    let done = wait_for_terminal(&app, &user, &start.id, Duration::from_secs(120)).await;
    assert_eq!(done.state, IndexerSearchState::Completed, "{done:?}");

    let healthy_view = indexer_view(&done, "healthy-a");
    assert_eq!(
        healthy_view.status,
        IndexerSearchIndexerStatus::Ok,
        "{healthy_view:?}"
    );
    assert!(
        healthy_view.elapsed_ms.is_some(),
        "per-indexer timing is recorded: {healthy_view:?}"
    );

    let broken_view = indexer_view(&done, "broken-b");
    assert_eq!(
        broken_view.status,
        IndexerSearchIndexerStatus::Failed,
        "{broken_view:?}"
    );
    let reason = broken_view
        .failure_reason
        .clone()
        .expect("failed indexer carries a reason");
    assert!(
        ["timeout", "auth", "rate limited", "error", "partial", "upstream failure"]
            .contains(&reason.as_str())
            || reason.starts_with("http "),
        "failure reason should be a short, stable word, got {reason:?}"
    );

    assert_eq!(
        done.releases.len(),
        1,
        "the healthy indexer's release survives: {done:?}"
    );
    assert_eq!(done.releases[0].indexer_id, "healthy-a");
    assert_eq!(done.totals.matched, 1);
    let healthy_release_id = done.releases[0].id.clone();

    // The raw kind must reach the plugin as a plain text query.
    let requests = healthy
        .received_requests()
        .await
        .expect("recorded requests on the healthy indexer");
    let search_requests = requests
        .iter()
        .map(|request| request.url.to_string())
        .filter(|url| url.contains("q="))
        .collect::<Vec<_>>();
    assert!(
        !search_requests.is_empty(),
        "expected a q= search request, saw: {:?}",
        requests
            .iter()
            .map(|request| request.url.to_string())
            .collect::<Vec<_>>()
    );
    assert!(
        search_requests
            .iter()
            .any(|url| url.contains("t=search") && !url.contains("t=movie")),
        "raw kind must issue a facet-less text search, saw: {search_requests:?}"
    );

    // Retry: only the failed indexer is re-dispatched, and the healthy
    // indexer's rows survive untouched.
    //
    // A 5xx escalates the search client's own system backoff, so the failed
    // indexer reports as backed-off rather than being re-queried until that
    // window expires. The healing merge itself is covered at the app layer in
    // `scryer-application`'s `lib_tests::indexer_search`, which is not subject
    // to the client's backoff clock.
    broken.reset().await;
    mount_healthy(&broken, "Paperman.2012.720p.WEB-DL-GRP", "ok-2").await;
    let healthy_requests_before = healthy
        .received_requests()
        .await
        .map(|requests| requests.len())
        .unwrap_or_default();

    app.retry_indexer_search(&user, &start.id)
        .await
        .expect("retry indexer search");
    let retried = wait_for_terminal(&app, &user, &start.id, Duration::from_secs(120)).await;
    assert_eq!(retried.state, IndexerSearchState::Completed, "{retried:?}");
    assert_eq!(
        indexer_view(&retried, "healthy-a").status,
        IndexerSearchIndexerStatus::Ok,
        "the healthy indexer keeps its earlier outcome: {retried:?}"
    );
    assert_eq!(
        healthy
            .received_requests()
            .await
            .map(|requests| requests.len())
            .unwrap_or_default(),
        healthy_requests_before,
        "retry must not re-query a healthy indexer"
    );

    let broken_after = indexer_view(&retried, "broken-b");
    assert_eq!(
        broken_after.status,
        IndexerSearchIndexerStatus::Skipped,
        "a backed-off indexer is stated as skipped, never as a silent empty ok: {broken_after:?}"
    );
    assert_eq!(
        broken_after.failure_reason.as_deref(),
        Some("temporarily backed off")
    );

    assert_eq!(
        retried.releases.len(),
        1,
        "the healthy indexer's row is neither dropped nor duplicated: {retried:?}"
    );
    let ids = retried
        .releases
        .iter()
        .map(|release| release.id.clone())
        .collect::<HashSet<_>>();
    assert_eq!(ids.len(), 1, "merge dedupes on release id: {retried:?}");
    assert!(
        ids.contains(&healthy_release_id),
        "the healthy indexer's release keeps its id across the retry"
    );
}
