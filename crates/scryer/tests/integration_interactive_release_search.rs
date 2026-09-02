//! Interactive release-search job integration tests (hotfix 0.17.1).
//!
//! Scenarios 1–4 drive the app-layer job methods directly against the full
//! production-parity search pipeline — discovery → MultiIndexerSearchClient
//! with the REAL `InMemoryUpstreamScheduler` (the shared TestContext uses the
//! Null scheduler) → real WASM newznab plugin → wiremock. Scenario 5 is a
//! GraphQL smoke test over the shared TestContext.

#![recursion_limit = "256"]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{TestContext, disable_platform_keystore_for_tests, initialize_wasm_runtime_for_tests};
use scryer_application::{
    AppServices, AppUseCase, FacetRegistry, INDEXER_ROUTING_SETTINGS_KEY, IndexerPluginProvider,
    InteractiveReleaseSearchIndexerStatus, InteractiveReleaseSearchRequest,
    InteractiveReleaseSearchSnapshot, InteractiveReleaseSearchState, InteractiveSearchKind,
    JwtAuthConfig, LibraryRootDraft, MovieFacetHandler, QUALITY_PROFILE_CATALOG_KEY,
    QUALITY_PROFILE_ID_KEY, SETTINGS_SCOPE_SYSTEM, SaveQualityProfileSettings, SeriesFacetHandler,
};
use scryer_domain::{ExternalId, IndexerConfig, MediaFacet, NewTitle, User};
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
use serde_json::{Value, json};
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

    // A live (empty) NZBGet mock: the per-batch result evaluation queries the
    // download client queue/history, and a dead endpoint would add seconds of
    // connect timeouts to every indexer batch, skewing timing assertions.
    let nzbget_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"version":"1.1","result":[]}"#),
        )
        .mount(&nzbget_server)
        .await;
    let nzbget_url = nzbget_server.uri();
    // Leak the mock server guard so the endpoint outlives setup.
    std::mem::forget(nzbget_server);

    scryer_application::DownloadClientConfigRepository::create(
        &*download_client_config_store,
        scryer_domain::DownloadClientConfig {
            id: "nzbget-1".into(),
            name: "NZBGet".into(),
            client_type: "nzbget".into(),
            config_json: serde_json::json!({
                "base_url": nzbget_url
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

    let staged_nzb_dir = tempfile::TempDir::new().expect("failed to create staged nzb tempdir");
    let staged_nzb_store = Arc::new(
        scryer_infrastructure_acquisition::downloads::staged_nzb_store::FileSystemStagedNzbStore::new(staged_nzb_dir.path())
            .await
            .expect("staged nzb store"),
    );
    // Leak the tempdir guard so the store outlives setup.
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
        app: scryer_domain::AppPermissionMask::from_permissions([
            scryer_domain::AppPermission::ManageCatalogSettings,
            // The Indexers-page gate a title-less query subject is held to (D13).
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

async fn add_movie(app: &AppUseCase, user: &User, name: &str, imdb: &str) -> String {
    app.add_title(
        user,
        NewTitle {
            name: name.to_string(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec![],
            external_ids: vec![ExternalId {
                source: "imdb".to_string(),
                value: imdb.to_string(),
            }],
            ..Default::default()
        },
    )
    .await
    .expect("add title")
    .id
}

fn title_request(title_id: &str) -> InteractiveReleaseSearchRequest {
    InteractiveReleaseSearchRequest {
        title_id: Some(title_id.to_string()),
        ..InteractiveReleaseSearchRequest::default()
    }
}

fn raw_query_request(query: &str) -> InteractiveReleaseSearchRequest {
    InteractiveReleaseSearchRequest {
        query: Some(query.to_string()),
        kind: Some(InteractiveSearchKind::Raw),
        ..InteractiveReleaseSearchRequest::default()
    }
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

async fn mount_delayed(server: &MockServer, title: &str, guid: &str, delay: Duration) {
    Mock::given(method("GET"))
        .and(path("/api"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(delay)
                .set_body_string(newznab_response_with_title(title, guid)),
        )
        .mount(server)
        .await;
}

async fn mount_rate_limited(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "2")
                .set_body_string("Request limit reached"),
        )
        .mount(server)
        .await;
}

async fn request_count(server: &MockServer) -> usize {
    server.received_requests().await.unwrap_or_default().len()
}

async fn wait_for_request_or_terminal(
    server: &MockServer,
    app: &AppUseCase,
    user: &User,
    job_id: &str,
    deadline: Duration,
) {
    let started = Instant::now();
    while started.elapsed() < deadline {
        if request_count(server).await > 0 {
            return;
        }
        let snapshot = app
            .interactive_release_search(user, job_id)
            .await
            .expect("read interactive search")
            .expect("interactive search should remain visible");
        assert_eq!(
            snapshot.state,
            InteractiveReleaseSearchState::Running,
            "interactive search ended before its indexer request reached the test server: {snapshot:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    panic!("expected the indexer request to reach the test server within {deadline:?}");
}

fn indexer_status<'a>(
    snapshot: &'a InteractiveReleaseSearchSnapshot,
    indexer_id: &str,
) -> &'a scryer_application::InteractiveReleaseSearchIndexerView {
    snapshot
        .indexers
        .iter()
        .find(|indexer| indexer.indexer_id == indexer_id)
        .unwrap_or_else(|| panic!("indexer {indexer_id} missing from snapshot: {snapshot:?}"))
}

/// Poll the job until `predicate` holds or `deadline` elapses; returns the
/// last snapshot either way so callers assert with full context.
async fn wait_for_snapshot(
    app: &AppUseCase,
    user: &User,
    job_id: &str,
    deadline: Duration,
    predicate: impl Fn(&InteractiveReleaseSearchSnapshot) -> bool,
) -> InteractiveReleaseSearchSnapshot {
    let started = Instant::now();
    loop {
        let snapshot = app
            .interactive_release_search(user, job_id)
            .await
            .expect("poll interactive release search")
            .expect("job should be present in registry");
        if predicate(&snapshot) || started.elapsed() > deadline {
            return snapshot;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// ── 1. Fast indexer streams in while a slow one is still searching ────────

#[tokio::test]
async fn fast_indexer_results_stream_in_before_slow_indexer_completes() {
    let fast = MockServer::start().await;
    mount_healthy(&fast, "Paperman.2012.1080p.WEB-DL-FASTGRP", "fast-1").await;
    let slow = MockServer::start().await;
    mount_healthy(&slow, "Paperman.2012.720p.WEB-DL-SLOWGRP", "slow-1").await;

    let now = chrono::Utc::now();
    let (app, user) = setup_app(vec![
        indexer_config("fast-a", format!("{}/api", fast.uri()), "key-a", now),
        indexer_config("slow-b", format!("{}/api", slow.uri()), "key-b", now),
    ])
    .await;
    let title_id = add_movie(&app, &user, "Paperman", "tt2388725").await;

    // Warm up the search pipeline (per-indexer WASM plugin workers compile
    // lazily on first use) so the "early partial" timing below measures the
    // job, not one-time plugin compilation.
    app.search_indexers_for_title(
        &user,
        title_id.clone(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("warmup search");

    // Now make the slow indexer slow. The delay must stay comfortably under
    // the 12s per-request indexer search timeout or the slow indexer times
    // out and finishes Failed instead of Completed.
    slow.reset().await;
    mount_delayed(
        &slow,
        "Paperman.2012.720p.WEB-DL-SLOWGRP",
        "slow-1",
        Duration::from_secs(8),
    )
    .await;

    let start = app
        .start_interactive_release_search(&user, title_request(&title_id))
        .await
        .expect("start job");
    assert_eq!(start.state, InteractiveReleaseSearchState::Running);
    assert_eq!(start.indexers.len(), 2);

    // Early partial: the fast indexer's release lands while the slow one is
    // still searching.
    let early = wait_for_snapshot(&app, &user, &start.id, Duration::from_secs(5), |snapshot| {
        !snapshot.results.is_empty()
    })
    .await;
    assert!(
        !early.results.is_empty(),
        "expected an early partial result: {early:?}"
    );
    assert_eq!(early.state, InteractiveReleaseSearchState::Running);
    assert_eq!(
        indexer_status(&early, "fast-a").status,
        InteractiveReleaseSearchIndexerStatus::Completed
    );
    assert_eq!(
        indexer_status(&early, "slow-b").status,
        InteractiveReleaseSearchIndexerStatus::Searching
    );

    let done = wait_for_snapshot(
        &app,
        &user,
        &start.id,
        Duration::from_secs(90),
        |snapshot| snapshot.state != InteractiveReleaseSearchState::Running,
    )
    .await;
    assert_eq!(done.state, InteractiveReleaseSearchState::Completed);
    assert!(done.completed_at.is_some());
    assert_eq!(
        indexer_status(&done, "fast-a").status,
        InteractiveReleaseSearchIndexerStatus::Completed
    );
    assert_eq!(
        indexer_status(&done, "slow-b").status,
        InteractiveReleaseSearchIndexerStatus::Completed
    );
    assert_eq!(done.results.len(), 2, "merged results: {done:?}");
}

// ── 2. A rate-limited indexer fails visibly without blanking the batch ────

#[tokio::test]
async fn rate_limited_indexer_is_marked_failed_and_healthy_results_survive() {
    let healthy = MockServer::start().await;
    mount_healthy(&healthy, "Paperman.2012.1080p.WEB-DL-GRP", "ok-1").await;
    let limited = MockServer::start().await;
    mount_healthy(&limited, "Paperman.2012.1080p.WEB-DL-GRP", "warmup-1").await;

    let now = chrono::Utc::now();
    let (app, user) = setup_app(vec![
        indexer_config("healthy-a", format!("{}/api", healthy.uri()), "key-a", now),
        indexer_config("limited-b", format!("{}/api", limited.uri()), "key-b", now),
    ])
    .await;
    let title_id = add_movie(&app, &user, "Paperman", "tt2388725").await;

    // Warm up the search pipeline so cold WASM compilation (~6s per indexer in
    // debug, worse under a parallel test sweep) does not eat into the job's
    // workflow deadline. The rate-limit response must belong to the interactive
    // job itself: priming it here persists a cooldown and prevents dispatch.
    app.search_indexers_for_title(
        &user,
        title_id.clone(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("warmup search");

    limited.reset().await;
    mount_rate_limited(&limited).await;

    let start = app
        .start_interactive_release_search(&user, title_request(&title_id))
        .await
        .expect("start job");

    // The pipeline is warm and the fixture's cooldown is bounded, so this is
    // ample time on a saturated CI host without making the test sleep through
    // the production 120-second indexer budget.
    let done = wait_for_snapshot(
        &app,
        &user,
        &start.id,
        Duration::from_secs(90),
        |snapshot| snapshot.state != InteractiveReleaseSearchState::Running,
    )
    .await;
    assert_eq!(done.state, InteractiveReleaseSearchState::Completed);
    assert!(
        done.results
            .iter()
            .any(|result| result.title.contains("Paperman")),
        "healthy indexer's results should survive: {done:?}"
    );
    let limited_view = indexer_status(&done, "limited-b");
    assert_eq!(
        limited_view.status,
        InteractiveReleaseSearchIndexerStatus::Failed
    );
    let reason = limited_view
        .failure_reason
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(
        reason.contains("limit") || reason.contains("429") || reason.contains("rate"),
        "failure reason should mention rate limiting: {limited_view:?}"
    );
    let healthy_view = indexer_status(&done, "healthy-a");
    assert_eq!(
        healthy_view.status,
        InteractiveReleaseSearchIndexerStatus::Completed,
        "healthy indexer should complete: {healthy_view:?}"
    );
}

// ── 3. Cancel mid-flight stops the job and outbound traffic ───────────────

#[tokio::test]
async fn cancel_mid_flight_stops_job_and_outbound_requests() {
    let slow = MockServer::start().await;
    mount_healthy(&slow, "Paperman.2012.1080p.WEB-DL-GRP", "slow-1").await;

    let now = chrono::Utc::now();
    let (app, user) = setup_app(vec![indexer_config(
        "slow-a",
        format!("{}/api", slow.uri()),
        "key-a",
        now,
    )])
    .await;
    let title_id = add_movie(&app, &user, "Paperman", "tt2388725").await;

    // Compile and initialize the real WASM plugin worker before starting the
    // timed job. Under CI-wide test contention, doing this lazily inside the
    // job can delay its first outbound request beyond the assertion window.
    app.search_indexers_for_title(
        &user,
        title_id.clone(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("warmup search");

    slow.reset().await;
    mount_delayed(
        &slow,
        "Paperman.2012.1080p.WEB-DL-GRP",
        "slow-1",
        Duration::from_secs(6),
    )
    .await;

    let start = app
        .start_interactive_release_search(&user, title_request(&title_id))
        .await
        .expect("start job");
    // Cancel only after the delayed request is in flight.
    wait_for_request_or_terminal(&slow, &app, &user, &start.id, Duration::from_secs(10)).await;

    let cancelled = app
        .cancel_interactive_release_search(&user, &start.id)
        .await
        .expect("cancel job");
    assert!(cancelled, "running job should accept the cancel");

    let snapshot = wait_for_snapshot(&app, &user, &start.id, Duration::from_secs(5), |snapshot| {
        snapshot.state == InteractiveReleaseSearchState::Cancelled
    })
    .await;
    assert_eq!(snapshot.state, InteractiveReleaseSearchState::Cancelled);
    assert!(snapshot.completed_at.is_some());

    // Cancelling twice is a no-op, not an error.
    let again = app
        .cancel_interactive_release_search(&user, &start.id)
        .await
        .expect("second cancel");
    assert!(!again);

    // No further outbound requests after the cancel settles.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let settled = request_count(&slow).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        request_count(&slow).await,
        settled,
        "request count should stop growing after cancel"
    );
}

// ── 4. Same-scope restart cancels the previous job ────────────────────────

#[tokio::test]
async fn same_scope_restart_cancels_previous_job() {
    let slow = MockServer::start().await;
    mount_delayed(
        &slow,
        "Paperman.2012.1080p.WEB-DL-GRP",
        "slow-1",
        Duration::from_secs(5),
    )
    .await;

    let now = chrono::Utc::now();
    let (app, user) = setup_app(vec![indexer_config(
        "slow-a",
        format!("{}/api", slow.uri()),
        "key-a",
        now,
    )])
    .await;
    let title_id = add_movie(&app, &user, "Paperman", "tt2388725").await;

    let first = app
        .start_interactive_release_search(&user, title_request(&title_id))
        .await
        .expect("start first job");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let second = app
        .start_interactive_release_search(&user, title_request(&title_id))
        .await
        .expect("start second job");
    assert_ne!(first.id, second.id);
    assert_eq!(second.state, InteractiveReleaseSearchState::Running);

    let first_snapshot = app
        .interactive_release_search(&user, &first.id)
        .await
        .expect("poll first job")
        .expect("first job still registered");
    assert_eq!(
        first_snapshot.state,
        InteractiveReleaseSearchState::Cancelled,
        "same-scope restart should cancel the first job"
    );

    let cancelled = app
        .cancel_interactive_release_search(&user, &second.id)
        .await
        .expect("cancel second job");
    assert!(cancelled);
}

// ── 5. A raw query subject issues a facet-less text search ────────────────

#[tokio::test]
async fn raw_query_subject_issues_a_text_search_and_completes_with_the_release() {
    let indexer = MockServer::start().await;
    mount_healthy(&indexer, "Paperman.2012.1080p.WEB-DL-GRP", "raw-1").await;

    let now = chrono::Utc::now();
    let (app, user) = setup_app(vec![indexer_config(
        "raw-a",
        format!("{}/api", indexer.uri()),
        "key-a",
        now,
    )])
    .await;

    let start = app
        .start_interactive_release_search(&user, raw_query_request("Paperman"))
        .await
        .expect("start raw query job");
    assert_eq!(start.state, InteractiveReleaseSearchState::Running);
    assert_eq!(start.indexers.len(), 1, "{start:?}");

    let done = wait_for_snapshot(
        &app,
        &user,
        &start.id,
        Duration::from_secs(120),
        |snapshot| snapshot.state != InteractiveReleaseSearchState::Running,
    )
    .await;
    assert_eq!(done.state, InteractiveReleaseSearchState::Completed, "{done:?}");
    assert!(
        done.results
            .iter()
            .any(|result| result.title.contains("Paperman")),
        "the raw query returns the indexer's release: {done:?}"
    );
    let view = indexer_status(&done, "raw-a");
    assert_eq!(view.status, InteractiveReleaseSearchIndexerStatus::Completed);
    assert!(
        view.elapsed_ms.is_some(),
        "per-indexer timing is recorded: {view:?}"
    );

    // A query subject with no facet must reach the plugin as plain text.
    let requests = indexer
        .received_requests()
        .await
        .expect("recorded requests on the indexer");
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
}

// ── 6. GraphQL smoke: start → poll → cancel-after-completion ──────────────

async fn schema_exec(ctx: &TestContext, query: &str, user: &User) -> Value {
    let req = async_graphql::Request::new(query).data(user.clone());
    let resp = ctx.schema.execute(req).await;
    serde_json::to_value(&resp).expect("serialize gql response")
}

fn assert_no_errors(body: &Value) {
    assert!(
        body.get("errors").is_none() || body["errors"].is_null(),
        "unexpected GraphQL errors: {body}"
    );
}

#[tokio::test]
async fn graphql_start_poll_and_cancel_after_completion() {
    let ctx = TestContext::new().await;

    let mut user = User {
        id: "interactive-search-user".into(),
        username: "interactive".into(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    user.authorization = scryer_domain::UserAuthorization {
        default_library: scryer_domain::LibraryPermissionMask::from_permissions([
            scryer_domain::LibraryPermission::View,
            scryer_domain::LibraryPermission::ManageTitles,
        ]),
        actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
        loaded: true,
        ..Default::default()
    };
    let title_id = add_movie(&ctx.app, &user, "Paperman", "tt2388725").await;

    let start_body = schema_exec(
        &ctx,
        &format!(
            r#"mutation {{
                startInteractiveReleaseSearch(input: {{ titleId: "{title_id}" }}) {{
                    id
                    state
                    results {{ title }}
                    indexers {{ indexerId name status resultCount failureReason }}
                    startedAt
                    completedAt
                }}
            }}"#
        ),
        &user,
    )
    .await;
    assert_no_errors(&start_body);
    let payload = &start_body["data"]["startInteractiveReleaseSearch"];
    assert_eq!(payload["state"], "RUNNING");
    let job_id = payload["id"].as_str().expect("job id").to_string();

    // Poll until the job reaches a terminal state (no indexers are configured
    // in the shared TestContext, so this settles almost immediately).
    let deadline = Instant::now() + Duration::from_secs(10);
    let final_state = loop {
        let poll_body = schema_exec(
            &ctx,
            &format!(
                r#"{{ interactiveReleaseSearch(id: "{job_id}") {{ id state results {{ title }} indexers {{ status }} completedAt }} }}"#
            ),
            &user,
        )
        .await;
        assert_no_errors(&poll_body);
        let job = &poll_body["data"]["interactiveReleaseSearch"];
        assert!(!job.is_null(), "job should be pollable: {poll_body}");
        assert_eq!(job["id"], json!(job_id));
        let state = job["state"].as_str().expect("state").to_string();
        if state != "RUNNING" || Instant::now() > deadline {
            break state;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(final_state, "COMPLETED");

    // Cancelling a finished job reports accepted=false, not an error.
    let cancel_body = schema_exec(
        &ctx,
        &format!(
            r#"mutation {{ cancelInteractiveReleaseSearch(id: "{job_id}") {{ id accepted }} }}"#
        ),
        &user,
    )
    .await;
    assert_no_errors(&cancel_body);
    assert_eq!(
        cancel_body["data"]["cancelInteractiveReleaseSearch"]["accepted"],
        json!(false)
    );

    // Unknown ids resolve to null rather than an error.
    let missing_body = schema_exec(
        &ctx,
        r#"{ interactiveReleaseSearch(id: "no-such-job") { id } }"#,
        &user,
    )
    .await;
    assert_no_errors(&missing_body);
    assert!(missing_body["data"]["interactiveReleaseSearch"].is_null());
}
