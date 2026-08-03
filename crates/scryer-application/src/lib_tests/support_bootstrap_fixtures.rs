use super::*;

pub(crate) fn bootstrap() -> (AppUseCase, User) {
    bootstrap_with_user_repo(Arc::new(MockUserRepo::default()))
}

pub(super) fn test_quality_profile(id: &str) -> QualityProfile {
    QualityProfile {
        id: id.to_string(),
        name: id.to_string(),
        criteria: QualityProfileCriteria::default(),
    }
}

pub(super) fn test_admin_user() -> User {
    let mut user = User::new_admin("admin");
    user.authorization = scryer_domain::UserAuthorization {
        app: AppPermissionMask::from_permissions([
            AppPermission::ManageUsers,
            AppPermission::ManagePermissions,
            AppPermission::ManageSystemSettings,
            AppPermission::ManageCatalogSettings,
        ]),
        default_library: scryer_domain::LibraryPermissionMask::from_permissions([
            scryer_domain::LibraryPermission::View,
            scryer_domain::LibraryPermission::ManageTitles,
            scryer_domain::LibraryPermission::ResolveImports,
            scryer_domain::LibraryPermission::ManageLibrary,
            scryer_domain::LibraryPermission::Request,
            scryer_domain::LibraryPermission::AutoApproveRequests,
        ]),
        actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
        loaded: true,
        ..Default::default()
    };
    user
}

pub(super) fn test_user_with_app_permissions(
    username: &str,
    app_permissions: AppPermissionMask,
) -> User {
    let mut user = User {
        id: Id::new().0,
        username: username.to_string(),
        password_hash: None,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    user.authorization.app = app_permissions;
    user.authorization.loaded = true;
    user
}

pub(super) async fn title_updated_events(app: &AppUseCase, title_id: &str) -> Vec<DomainEvent> {
    app.services
        .events
        .domain_events
        .list(&DomainEventFilter {
            event_types: Some(vec![DomainEventType::TitleUpdated]),
            title_id: Some(title_id.to_string()),
            facet: None,
            after_sequence: Some(0),
            before_sequence: None,
            limit: 100,
        })
        .await
        .expect("title updated events should load")
}

pub(super) fn bootstrap_with_user_repo(users: Arc<MockUserRepo>) -> (AppUseCase, User) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let settings = Arc::new(StoredSettingsRepo::default());
    let quality_profiles = Arc::new(MockQualityProfileRepo);
    let download_client = Arc::new(StubDownloadClient::default());
    let indexer_client = Arc::new(MockIndexerClient);

    let services = AppServices::builder(
        titles.clone(),
        shows,
        users,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts.clone(),
        settings,
        quality_profiles,
        String::new(),
    )
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .with_libraries(Arc::new(MockLibraryRepo::default()))
    .build_partial_for_tests();
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
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    (app, test_admin_user())
}

pub(super) struct MediaRequestTestHarness {
    pub(super) app: AppUseCase,
    pub(super) user: User,
    pub(super) manager: User,
    pub(super) titles: Arc<MockTitleRepo>,
    pub(super) libraries: Arc<MockLibraryRepo>,
    pub(super) media_requests: Arc<MockMediaRequestRepo>,
    pub(super) domain_events: Arc<MockDomainEventRepo>,
}

pub(super) fn bootstrap_media_request_app() -> MediaRequestTestHarness {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let settings = Arc::new(StoredSettingsRepo::default());
    let quality_profiles = Arc::new(MockQualityProfileRepo);
    let download_client = Arc::new(StubDownloadClient::default());
    let indexer_client = Arc::new(MockIndexerClient);
    let libraries = Arc::new(MockLibraryRepo::default());
    let domain_events = Arc::new(MockDomainEventRepo::default());
    let media_requests = Arc::new(MockMediaRequestRepo::with_domain_events(
        domain_events.clone(),
    ));
    let wanted_items = Arc::new(TrackingAcquisitionScopeStateRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let metadata_gateway = Arc::new(MockMetadataGateway {
        movies: (9000..9100)
            .map(|tvdb_id| (tvdb_id, make_movie_metadata(tvdb_id, "Glass Harbor")))
            .collect(),
    });

    let services = AppServices::builder(
        titles.clone(),
        shows,
        users,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts.clone(),
        settings,
        quality_profiles,
        String::new(),
    )
    .with_domain_events(domain_events.clone())
    .with_libraries(libraries.clone())
    .with_media_requests(media_requests.clone())
    .with_metadata_gateway(metadata_gateway)
    .with_acquisition_scope_states(wanted_items.clone())
    .with_pending_releases(pending_releases.clone())
    .with_download_submissions(download_submissions.clone())
    .with_blocklist_repo(Arc::new(MockBlocklistRepo::default()))
    .with_acquisition_state(Arc::new(TrackingAcquisitionStateRepo {
        download_submissions,
        pending_releases,
        acquisition_scope_states: wanted_items,
    }))
    .build_partial_for_tests();
    let mut registry = FacetRegistry::new();
    registry.register(Arc::new(MovieFacetHandler));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Series,
    )));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Anime,
    )));

    let mut requester = User::new_admin("requester");
    requester.authorization = scryer_domain::UserAuthorization {
        app: AppPermissionMask::MANAGE_CATALOG_SETTINGS,
        default_library: scryer_domain::LibraryPermissionMask::from_permissions([
            scryer_domain::LibraryPermission::Request,
        ]),
        actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
        loaded: true,
        ..Default::default()
    };

    MediaRequestTestHarness {
        app: AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "scryer-test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(registry),
        ),
        user: requester,
        manager: test_admin_user(),
        titles,
        libraries,
        media_requests,
        domain_events,
    }
}

pub(super) fn media_request_input(
    library_id: impl Into<String>,
    tvdb_id: i64,
) -> SubmitMediaRequestInput {
    SubmitMediaRequestInput {
        library_id: library_id.into(),
        facet: MediaFacet::Movie,
        title: "Glass Harbor".to_string(),
        sort_title: Some("Glass Harbor".to_string()),
        slug: Some("glass-harbor".to_string()),
        year: Some(2026),
        overview: Some("A test request subject".to_string()),
        runtime_minutes: Some(101),
        language: Some("en".to_string()),
        content_status: Some("Released".to_string()),
        requested_quality_profile_id: None,
        requested_monitor_type: None,
        external_ids: vec![
            ExternalId {
                source: "TVDB".to_string(),
                value: tvdb_id.to_string(),
            },
            ExternalId {
                source: "imdb".to_string(),
                value: "tt1234567".to_string(),
            },
        ],
    }
}

pub(super) fn library_permission_user(
    username: &str,
    library_id: &str,
    permissions: &[scryer_domain::LibraryPermission],
) -> User {
    library_permission_user_with_grants(username, &[(library_id, permissions)])
}

pub(super) fn library_permission_user_with_grants(
    username: &str,
    grants: &[(&str, &[scryer_domain::LibraryPermission])],
) -> User {
    let mut user = User::new_admin(username);
    user.authorization = scryer_domain::UserAuthorization {
        app: AppPermissionMask::NONE,
        libraries: grants
            .iter()
            .map(|(library_id, permissions)| {
                (
                    (*library_id).to_string(),
                    scryer_domain::LibraryPermissionMask::from_permissions(
                        permissions.iter().copied(),
                    ),
                )
            })
            .collect(),
        default_library: scryer_domain::LibraryPermissionMask::NONE,
        actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
        login_status: Default::default(),
        loaded: true,
    };
    user
}

pub(super) fn custom_movie_library(id: &str, name: &str) -> Library {
    let mut library = mock_default_library(MediaFacet::Movie);
    library.id = id.to_string();
    library.name = name.to_string();
    library.slug = name.to_ascii_lowercase().replace(' ', "-");
    library.is_default = false;
    library
}

pub(super) async fn wait_for_title_image_clear_calls(
    repo: &BlockingTitleImageRepo,
    expected: usize,
) {
    timeout(Duration::from_secs(1), async {
        loop {
            if repo.clear_calls.load(Ordering::SeqCst) >= expected {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("title image clear call count should reach expected value");
}

pub(super) async fn wait_for_title_image_cache_clear_idle(app: &AppUseCase) {
    timeout(Duration::from_secs(1), async {
        loop {
            if !app
                .runtime
                .catalog
                .title_image_cache_clear_scheduled
                .load(Ordering::Acquire)
            {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("title image cache clear should become idle");
}

pub(super) fn bootstrap_with_metadata_gateway_and_titles(
    metadata_gateway: Arc<dyn MetadataGateway>,
) -> (AppUseCase, User, Arc<MockTitleRepo>) {
    bootstrap_with_metadata_gateway_settings_and_titles(
        metadata_gateway,
        Arc::new(StoredSettingsRepo::default()),
    )
}

pub(super) fn bootstrap_with_metadata_gateway_settings_and_titles(
    metadata_gateway: Arc<dyn MetadataGateway>,
    settings: Arc<dyn SettingsRepository>,
) -> (AppUseCase, User, Arc<MockTitleRepo>) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let quality_profiles = Arc::new(MockQualityProfileRepo);
    let download_client = Arc::new(StubDownloadClient::default());
    let indexer_client = Arc::new(MockIndexerClient);

    let services = AppServices::builder(
        titles.clone(),
        shows,
        users,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts.clone(),
        settings,
        quality_profiles,
        String::new(),
    )
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .with_metadata_gateway(metadata_gateway)
    .with_libraries(Arc::new(MockLibraryRepo::default()))
    .build_partial_for_tests();

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
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    (app, test_admin_user(), titles)
}

pub(super) fn make_due_hydration_title(id: &str, facet: MediaFacet, tvdb_id: i64) -> Title {
    Title {
        id: id.to_string(),
        name: format!("Title {id}"),
        library_id: scryer_domain::default_library_id_for_facet(&facet),
        root_folder_id: scryer_domain::root_folder_id_for_path("/data/test"),
        facet,
        monitored: true,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: vec![ExternalId {
            source: "tvdb".to_string(),
            value: tvdb_id.to_string(),
        }],
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2026),
        overview: None,
        poster_url: None,
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

pub(super) fn make_movie_metadata(tvdb_id: i64, name: &str) -> MovieMetadata {
    MovieMetadata {
        target_key: None,
        tvdb_id,
        name: name.to_string(),
        slug: name.to_ascii_lowercase().replace(' ', "-"),
        year: Some(2026),
        content_status: "Released".to_string(),
        overview: format!("{name} overview"),
        poster_url: format!("https://example.com/{tvdb_id}.jpg"),
        background_url: None,
        language: "eng".to_string(),
        runtime_minutes: 100,
        sort_title: name.to_string(),
        imdb_id: format!("tt{tvdb_id:07}"),
        tmdb_id: None,
        popularity: None,
        anidb_id: None,
        canonical_tags: vec![],
        studio: "Test Studio".to_string(),
        tmdb_release_date: Some("2026-01-01".to_string()),
        ratings: Default::default(),
    }
}

pub(super) fn bootstrap_with_cleanup_tracking(
    download_client: Arc<StubDownloadClient>,
    download_submissions: Arc<TrackingDownloadSubmissionRepo>,
    pending_releases: Arc<TrackingPendingReleaseRepo>,
) -> (AppUseCase, User) {
    bootstrap_with_cleanup_tracking_and_indexer(
        download_client,
        download_submissions,
        pending_releases,
        Arc::new(MockIndexerClient),
    )
}

pub(super) fn bootstrap_with_cleanup_tracking_and_queue_commands(
    download_client: Arc<StubDownloadClient>,
    download_submissions: Arc<TrackingDownloadSubmissionRepo>,
    pending_releases: Arc<TrackingPendingReleaseRepo>,
    download_queue_commands: Arc<TrackingDownloadQueueCommandRepo>,
) -> (AppUseCase, User) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let settings = Arc::new(StoredSettingsRepo::default());
    let quality_profiles = Arc::new(MockQualityProfileRepo);

    let services = AppServices::builder(
        titles,
        shows,
        users,
        indexer_configs,
        Arc::new(MockIndexerClient),
        download_client,
        download_client_configs,
        release_attempts,
        settings,
        quality_profiles,
        String::new(),
    )
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .with_download_submissions(download_submissions)
    .with_pending_releases(pending_releases)
    .with_download_queue_commands(download_queue_commands)
    .with_blocklist_repo(Arc::new(MockBlocklistRepo::default()))
    .with_libraries(Arc::new(MockLibraryRepo::default()))
    .build_partial_for_tests();

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
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    (app, test_admin_user())
}

pub(super) fn bootstrap_with_cleanup_tracking_and_tracked_handle(
    download_client: Arc<StubDownloadClient>,
    download_submissions: Arc<TrackingDownloadSubmissionRepo>,
    pending_releases: Arc<TrackingPendingReleaseRepo>,
    tracked_download_handle: crate::tracked_downloads::TrackedDownloadHandle,
) -> (AppUseCase, User) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let settings = Arc::new(StoredSettingsRepo::default());
    let quality_profiles = Arc::new(MockQualityProfileRepo);

    let services = AppServices::builder(
        titles,
        shows,
        users,
        indexer_configs,
        Arc::new(MockIndexerClient),
        download_client,
        download_client_configs,
        release_attempts,
        settings,
        quality_profiles,
        String::new(),
    )
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .with_download_submissions(download_submissions)
    .with_pending_releases(pending_releases)
    .with_blocklist_repo(Arc::new(MockBlocklistRepo::default()))
    .with_tracked_download_handle(tracked_download_handle)
    .with_libraries(Arc::new(MockLibraryRepo::default()))
    .build_partial_for_tests();

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
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    (app, test_admin_user())
}

pub(super) fn bootstrap_with_cleanup_tracking_and_indexer(
    download_client: Arc<StubDownloadClient>,
    download_submissions: Arc<TrackingDownloadSubmissionRepo>,
    pending_releases: Arc<TrackingPendingReleaseRepo>,
    indexer_client: Arc<dyn IndexerClient>,
) -> (AppUseCase, User) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let settings = Arc::new(StoredSettingsRepo::default());
    let quality_profiles = Arc::new(MockQualityProfileRepo);

    let services = AppServices::builder(
        titles,
        shows,
        users,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts,
        settings,
        quality_profiles,
        String::new(),
    )
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .with_download_submissions(download_submissions)
    .with_pending_releases(pending_releases)
    .with_blocklist_repo(Arc::new(MockBlocklistRepo::default()))
    .with_libraries(Arc::new(MockLibraryRepo::default()))
    .build_partial_for_tests();

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
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    (app, test_admin_user())
}

pub(super) fn bootstrap_with_search_settings_and_indexer(
    settings: Arc<StoredSettingsRepo>,
    indexer_client: Arc<dyn IndexerClient>,
) -> (AppUseCase, User) {
    bootstrap_with_settings_repo_and_profiles(
        settings,
        Arc::new(MockQualityProfileRepo),
        indexer_client,
    )
}

pub(super) fn bootstrap_with_search_settings_indexer_and_configs(
    settings: Arc<StoredSettingsRepo>,
    indexer_client: Arc<dyn IndexerClient>,
    configs: Vec<IndexerConfig>,
) -> (AppUseCase, User) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo {
        store: Arc::new(Mutex::new(configs)),
    });
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let download_client = Arc::new(StubDownloadClient::default());
    let plugin_provider = Arc::new(MockIndexerPluginProvider {
        client: Arc::clone(&indexer_client),
    });

    let services = AppServices::builder(
        titles,
        shows,
        users,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts,
        settings,
        Arc::new(MockQualityProfileRepo),
        String::new(),
    )
    .with_plugin_provider(plugin_provider)
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .with_libraries(Arc::new(MockLibraryRepo::default()))
    .build_partial_for_tests();

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
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    (app, test_admin_user())
}

pub(super) fn bootstrap_with_settings_repo_and_profiles(
    settings: Arc<dyn SettingsRepository>,
    quality_profiles: Arc<dyn QualityProfileRepository>,
    indexer_client: Arc<dyn IndexerClient>,
) -> (AppUseCase, User) {
    bootstrap_with_settings_repo_and_profiles_and_libraries(
        settings,
        quality_profiles,
        indexer_client,
        Arc::new(MockLibraryRepo::default()),
    )
}

pub(super) fn bootstrap_with_settings_repo_and_profiles_and_libraries(
    settings: Arc<dyn SettingsRepository>,
    quality_profiles: Arc<dyn QualityProfileRepository>,
    indexer_client: Arc<dyn IndexerClient>,
    libraries: Arc<dyn LibraryRepository>,
) -> (AppUseCase, User) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let download_client = Arc::new(StubDownloadClient::default());
    let plugin_provider = Arc::new(MockIndexerPluginProvider {
        client: Arc::clone(&indexer_client),
    });

    let services = AppServices::builder(
        titles,
        shows,
        users,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts,
        settings,
        quality_profiles,
        String::new(),
    )
    .with_plugin_provider(plugin_provider)
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .with_libraries(libraries)
    .build_partial_for_tests();

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
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    (app, test_admin_user())
}

pub(super) fn synthetic_direct_nab_indexer_config(id: &str, provider_type: &str) -> IndexerConfig {
    IndexerConfig {
        id: id.to_string(),
        name: format!("Synthetic {provider_type}"),
        provider_type: provider_type.to_string(),
        base_url: "https://example.invalid".to_string(),
        api_key_encrypted: None,
        rate_limit_seconds: None,
        rate_limit_burst: None,
        disabled_until: None,
        is_enabled: true,
        enable_interactive_search: true,
        enable_auto_search: true,
        indexer_proxy_config_id: None,
        managed_parent_config_id: None,
        managed_child_key: None,
        managed_metadata_json: None,
        caps_snapshot_json: None,
        last_health_status: None,
        last_error_at: None,
        config_json: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

pub(super) fn bootstrap_with_cutoff_projection_state(
    settings: Arc<StoredSettingsRepo>,
    quality_profiles: Arc<StoredQualityProfileRepo>,
    media_files: Arc<MockMediaFileRepo>,
) -> (AppUseCase, User, Arc<MockTitleRepo>) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let download_client = Arc::new(StubDownloadClient::default());
    let indexer_client = Arc::new(MockIndexerClient);

    let services = AppServices::builder(
        titles.clone(),
        shows,
        users,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts,
        settings,
        quality_profiles,
        String::new(),
    )
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .with_media_files(media_files)
    .with_libraries(Arc::new(MockLibraryRepo::default()))
    .build_partial_for_tests();

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
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    (app, test_admin_user(), titles)
}

pub(super) fn bootstrap_with_delete_queue(
    download_client: Arc<StubDownloadClient>,
    download_queue_commands: Arc<TrackingDownloadQueueCommandRepo>,
) -> (AppUseCase, User) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let settings = Arc::new(MockSettingsRepo);
    let quality_profiles = Arc::new(MockQualityProfileRepo);
    let indexer_client = Arc::new(MockIndexerClient);

    let services = AppServices::builder(
        titles,
        shows,
        users,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts,
        settings,
        quality_profiles,
        String::new(),
    )
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .with_download_queue_commands(download_queue_commands)
    .with_libraries(Arc::new(MockLibraryRepo::default()))
    .build_partial_for_tests();

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
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    (app, test_admin_user())
}

pub(super) fn bootstrap_with_acquisition_tracking(
    download_client: Arc<StubDownloadClient>,
    download_submissions: Arc<TrackingDownloadSubmissionRepo>,
    pending_releases: Arc<TrackingPendingReleaseRepo>,
    acquisition_scope_states: Arc<TrackingAcquisitionScopeStateRepo>,
) -> (AppUseCase, User) {
    bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        download_submissions,
        pending_releases,
        acquisition_scope_states,
        Arc::new(MockIndexerClient),
    )
}

pub(super) fn bootstrap_with_acquisition_tracking_and_indexer(
    download_client: Arc<StubDownloadClient>,
    download_submissions: Arc<TrackingDownloadSubmissionRepo>,
    pending_releases: Arc<TrackingPendingReleaseRepo>,
    acquisition_scope_states: Arc<TrackingAcquisitionScopeStateRepo>,
    indexer_client: Arc<dyn IndexerClient>,
) -> (AppUseCase, User) {
    let (app, user, _) = bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
        download_client,
        download_submissions,
        pending_releases,
        acquisition_scope_states,
        indexer_client,
    );
    (app, user)
}

pub(super) fn bootstrap_with_acquisition_tracking_and_indexer_and_release_attempts(
    download_client: Arc<StubDownloadClient>,
    download_submissions: Arc<TrackingDownloadSubmissionRepo>,
    pending_releases: Arc<TrackingPendingReleaseRepo>,
    acquisition_scope_states: Arc<TrackingAcquisitionScopeStateRepo>,
    indexer_client: Arc<dyn IndexerClient>,
) -> (AppUseCase, User, Arc<MockReleaseAttemptRepo>) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    // The convergence cursor only searches a scope's routed
    // indexers, so a background cycle needs at least one enabled indexer routed
    // to it. Seed a synthetic direct-Newznab indexer the core `indexer_client`
    // fake answers for.
    let indexer_configs = Arc::new(MockIndexerConfigRepo {
        store: Arc::new(Mutex::new(vec![synthetic_direct_nab_indexer_config(
            "acquisition-indexer",
            "newznab",
        )])),
    });
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    download_client_configs
        .store
        .try_lock()
        .expect("download client config store should not be contended during bootstrap")
        .push(DownloadClientConfig {
            id: "background-search-default-client".to_string(),
            name: "Background Search Default Client".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 10_000,
            is_enabled: true,
            status: scryer_domain::DownloadClientStatus::Healthy,
            last_error: None,
            last_seen_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let settings = Arc::new(StoredSettingsRepo::default());
    let quality_profiles = Arc::new(MockQualityProfileRepo);

    let services = AppServices::builder(
        titles.clone(),
        shows,
        users,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts.clone(),
        settings,
        quality_profiles,
        String::new(),
    )
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .with_download_submissions(download_submissions.clone())
    .with_pending_releases(pending_releases.clone())
    .with_blocklist_repo(Arc::new(MockBlocklistRepo::default()))
    .with_libraries(Arc::new(MockLibraryRepo::default()))
    // The convergence cursor derives targets from library state.
    // With mock catalog stores, bridge the derivation to the seeded wanted
    // rows so `run_convergence_cycle_once` reaches each seeded monitored scope.
    .with_media_files(Arc::new(MockMediaFileRepo::with_missing_scope_source(
        acquisition_scope_states.clone(),
        titles,
    )))
    .build_partial_for_tests();

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
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );
    let app = app.with_test_overrides(|services| {
        services
            .with_acquisition_state(Arc::new(TrackingAcquisitionStateRepo {
                download_submissions,
                pending_releases,
                acquisition_scope_states: acquisition_scope_states.clone(),
            }))
            .with_acquisition_scope_states(acquisition_scope_states)
    });
    (app, test_admin_user(), release_attempts)
}

pub(super) fn bootstrap_with_scan_unmatched_tracking(
    settings: Arc<StoredSettingsRepo>,
    library_scanner: Arc<MutableLibraryScanner>,
    unmatched_items: Arc<TrackingLibraryScanUnmatchedItemRepo>,
) -> (AppUseCase, User) {
    let (app, user, _) = bootstrap_with_scan_unmatched_and_metadata_tracking_and_titles(
        settings,
        library_scanner,
        unmatched_items,
        Arc::new(EmptySearchMetadataGateway),
    );
    (app, user)
}

pub(super) fn bootstrap_with_library_delete_repositories(
    titles: Arc<MockTitleRepo>,
    settings: Arc<StoredSettingsRepo>,
    unmatched_items: Arc<TrackingLibraryScanUnmatchedItemRepo>,
    domain_events: Arc<dyn DomainEventRepository>,
    housekeeping: Arc<dyn HousekeepingRepository>,
    pending_releases: Arc<dyn PendingReleaseRepository>,
) -> (AppUseCase, User) {
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    download_client_configs
        .store
        .try_lock()
        .expect("download client config store should not be contended during bootstrap")
        .push(DownloadClientConfig {
            id: "default-download-client".to_string(),
            name: "Default Download Client".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
            status: scryer_domain::DownloadClientStatus::Healthy,
            last_error: None,
            last_seen_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let quality_profiles = Arc::new(MockQualityProfileRepo);
    let download_client = Arc::new(StubDownloadClient::default());
    let indexer_client = Arc::new(MockIndexerClient);
    let media_files = Arc::new(MockMediaFileRepo::default());

    let services = AppServices::builder(
        titles,
        shows,
        users,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts,
        settings,
        quality_profiles,
        String::new(),
    )
    .with_domain_events(domain_events)
    .with_metadata_gateway(Arc::new(EmptySearchMetadataGateway))
    .with_library_scanner(Arc::new(MutableLibraryScanner::default()))
    .with_media_files(media_files)
    .with_library_scan_unmatched_items(unmatched_items)
    .with_pending_releases(pending_releases)
    .with_housekeeping(housekeeping)
    .with_libraries(Arc::new(MockLibraryRepo::default()))
    .build_partial_for_tests();

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
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    (app, test_admin_user())
}

pub(super) fn bootstrap_with_scan_unmatched_and_metadata_tracking(
    settings: Arc<StoredSettingsRepo>,
    library_scanner: Arc<MutableLibraryScanner>,
    unmatched_items: Arc<TrackingLibraryScanUnmatchedItemRepo>,
    metadata_gateway: Arc<dyn MetadataGateway>,
) -> (AppUseCase, User) {
    let (app, user, _) = bootstrap_with_scan_unmatched_and_metadata_tracking_and_titles(
        settings,
        library_scanner,
        unmatched_items,
        metadata_gateway,
    );
    (app, user)
}

pub(super) fn bootstrap_with_scan_unmatched_and_metadata_tracking_and_titles(
    settings: Arc<StoredSettingsRepo>,
    library_scanner: Arc<MutableLibraryScanner>,
    unmatched_items: Arc<TrackingLibraryScanUnmatchedItemRepo>,
    metadata_gateway: Arc<dyn MetadataGateway>,
) -> (AppUseCase, User, Arc<MockTitleRepo>) {
    let titles = Arc::new(MockTitleRepo {
        pending_import_items: Some(unmatched_items.items.clone()),
        ..Default::default()
    });
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let quality_profiles = Arc::new(MockQualityProfileRepo);
    let download_client = Arc::new(StubDownloadClient::default());
    let indexer_client = Arc::new(MockIndexerClient);
    let media_files = Arc::new(MockMediaFileRepo::default());

    let services = AppServices::builder(
        titles.clone(),
        shows,
        users,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts,
        settings,
        quality_profiles,
        String::new(),
    )
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .with_metadata_gateway(metadata_gateway)
    .with_library_scanner(library_scanner)
    .with_media_files(media_files)
    .with_library_scan_unmatched_items(unmatched_items)
    .with_libraries(Arc::new(MockLibraryRepo::default()))
    .build_partial_for_tests();

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
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    (app, test_admin_user(), titles)
}

pub(super) struct FixedBatchSearchMetadataGateway {
    pub(super) results: Vec<MetadataSearchItem>,
}

#[async_trait]
impl MetadataGateway for FixedBatchSearchMetadataGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Ok(self.results.clone())
    }

    async fn search_tvdb_batch(
        &self,
        queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        Ok(queries
            .iter()
            .cloned()
            .map(|query| (query, self.results.clone()))
            .collect())
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        Ok(Vec::new())
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Ok(MultiMetadataSearchResult::default())
    }

    async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        Err(AppError::NotFound(
            "movie metadata unavailable in test".into(),
        ))
    }

    async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        Err(AppError::NotFound(
            "series metadata unavailable in test".into(),
        ))
    }

    async fn get_metadata_bulk(
        &self,
        _movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        Ok(BulkMetadataResult {
            movies: HashMap::new(),
            series: HashMap::new(),
        })
    }
}

pub(super) fn build_test_library_file(path: &str) -> LibraryFile {
    LibraryFile {
        path: path.to_string(),
        display_name: Path::new(path)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string(),
        nfo_path: None,
        size_bytes: None,
        source_signature_scheme: None,
        source_signature_value: None,
    }
}

pub(super) fn build_test_library_files(paths: &[&Path]) -> Vec<LibraryFile> {
    paths
        .iter()
        .map(|path| build_test_library_file(path.to_string_lossy().as_ref()))
        .collect()
}

pub(super) async fn bootstrap_movie_scan_app(
    root: &Path,
    library_files: Vec<LibraryFile>,
    metadata_gateway: Arc<dyn MetadataGateway>,
) -> (AppUseCase, User, Arc<TrackingLibraryScanUnmatchedItemRepo>) {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            root.to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner.set_library_files(library_files).await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        unmatched_items.clone(),
        metadata_gateway,
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile movie root");

    (app, user, unmatched_items)
}

pub(super) async fn create_movie_title_with_folder(
    app: &AppUseCase,
    user: &User,
    name: &str,
    folder_path: &Path,
) -> Title {
    let title = app
        .add_title(
            user,
            NewTitle {
                name: name.into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create movie title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, folder_path.to_string_lossy().as_ref())
        .await
        .expect("set movie folder path");
    app.services
        .catalog
        .titles
        .get_by_id(&title.id)
        .await
        .expect("load movie title")
        .expect("movie title exists")
}

pub(super) fn media_file_role_for_path(files: &[TitleMediaFile], path: &Path) -> MediaFileRole {
    let path = path.to_string_lossy();
    files
        .iter()
        .find(|file| file.file_path == path)
        .map(|file| file.role)
        .expect("media file role")
}

pub(super) fn build_test_unmatched_item(
    id: &str,
    facet: MediaFacet,
    scan_root: &str,
    item_path: &str,
    display_name: &str,
    query: &str,
    year_hint: Option<i32>,
) -> LibraryScanUnmatchedItem {
    let timestamp = chrono::Utc::now().to_rfc3339();
    LibraryScanUnmatchedItem {
        id: id.to_string(),
        library_id: scryer_domain::default_library_id_for_facet(&facet),
        facet,
        status: PendingImportStatus::Pending,
        title_id: None,
        scan_session_id: "scan-session-1".to_string(),
        scan_root: scan_root.to_string(),
        item_path: item_path.to_string(),
        display_name: display_name.to_string(),
        query: query.to_string(),
        year_hint,
        reason_code: "no_metadata_match".to_string(),
        error_message: None,
        search_attempts: vec![],
        created_at: timestamp.clone(),
        updated_at: timestamp,
    }
}

pub(super) fn build_root_folder_entry(path: &Path, is_default: bool) -> RootFolderEntry {
    RootFolderEntry {
        path: path.to_string_lossy().to_string(),
        is_default,
    }
}

pub(super) async fn wait_for_projected_library_scan_session_matching<F>(
    app: &AppUseCase,
    session_id: &str,
    predicate: F,
) -> LibraryScanSession
where
    F: Fn(&LibraryScanSession) -> bool,
{
    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        if let Some(session) =
            crate::library_scan_coordinator::load_projected_library_scan_session(app, session_id)
                .await
                .expect("projected library scan session")
            && predicate(&session)
        {
            return session;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for projected session {session_id} to satisfy predicate",
        );
        sleep(Duration::from_millis(10)).await;
    }
}

pub(super) fn empty_update_media_settings_with_roots(
    root_folders: Vec<RootFolderEntry>,
) -> UpdateMediaSettings {
    UpdateMediaSettings {
        library_path: None,
        root_folders: Some(root_folders),
        required_audio_languages: None,
        folder_template: None,
        season_folder_template: None,
        specials_folder_template: None,
        rename_enabled: None,
        rename_template: None,
        rename_collision_policy: None,
        rename_missing_metadata_policy: None,
        filler_policy: None,
        recap_policy: None,
        monitor_specials: None,
        inter_season_movies: None,
        monitor_filler_movies: None,
        nfo_write_on_import: None,
        plexmatch_write_on_import: None,
        import_mode: None,
        set_permissions_linux: None,
        file_chmod: None,
        folder_chmod: None,
        chown_group: None,
    }
}

pub(super) fn empty_update_media_settings() -> UpdateMediaSettings {
    UpdateMediaSettings {
        library_path: None,
        root_folders: None,
        required_audio_languages: None,
        folder_template: None,
        season_folder_template: None,
        specials_folder_template: None,
        rename_enabled: None,
        rename_template: None,
        rename_collision_policy: None,
        rename_missing_metadata_policy: None,
        filler_policy: None,
        recap_policy: None,
        monitor_specials: None,
        inter_season_movies: None,
        monitor_filler_movies: None,
        nfo_write_on_import: None,
        plexmatch_write_on_import: None,
        import_mode: None,
        set_permissions_linux: None,
        file_chmod: None,
        folder_chmod: None,
        chown_group: None,
    }
}

pub(super) fn empty_library_settings_override() -> LibrarySettingsOverrideDraft {
    LibrarySettingsOverrideDraft {
        required_audio_languages: None,
        quality_profile_id: None,
        request_quality_profile_ids: None,
        scoring_persona: None,
        filler_policy: None,
        recap_policy: None,
        monitor_specials: None,
        inter_season_movies: None,
        monitor_filler_movies: None,
        nfo_write_on_import: None,
        plexmatch_write_on_import: None,
        import_mode: None,
        set_permissions_linux: None,
        file_chmod: None,
        folder_chmod: None,
        chown_group: None,
        indexer_routing: None,
        download_client_routing: None,
    }
}

pub(super) fn test_series_movie_link(
    title_id: &str,
    name: &str,
    year: Option<i32>,
    imdb_id: Option<&str>,
    tvdb_id: Option<&str>,
) -> scryer_domain::SeriesMovieLink {
    let now = Utc::now();
    scryer_domain::SeriesMovieLink {
        id: Id::new().0,
        series_title_id: title_id.to_string(),
        movie: scryer_domain::MovieEntity {
            id: Id::new().0,
            title: name.to_string(),
            sort_title: Some(name.to_string()),
            slug: Some(name.to_ascii_lowercase().replace(' ', "-")),
            year,
            overview: Some("Series movie".to_string()),
            poster_url: None,
            background_url: None,
            language: Some("ja".to_string()),
            runtime_minutes: Some(110),
            content_status: Some("released".to_string()),
            studio: Some("Studio".to_string()),
            digital_release_date: Some("2024-02-01".to_string()),
            imdb_id: imdb_id.map(str::to_string),
            tvdb_id: tvdb_id.map(str::to_string),
            tmdb_id: None,
            mal_id: None,
            anidb_id: None,
            created_at: now,
            updated_at: now,
        },
        placement: Some("between_seasons".to_string()),
        narrative_order: Some("1.1".to_string()),
        after_season: Some(1),
        before_season: None,
        linked_episode_id: None,
        association_confidence: Some("high".to_string()),
        continuity_status: Some("canon".to_string()),
        movie_form: Some("movie".to_string()),
        confidence: None,
        signal_summary: None,
        source: Some("test".to_string()),
        monitored: true,
        legacy_collection_id: None,
        created_at: now,
        updated_at: now,
    }
}

pub(super) fn cutoff_projection_test_profile(id: &str, cutoff_tier: &str) -> QualityProfile {
    QualityProfile {
        id: id.to_string(),
        name: format!("Profile {id}"),
        criteria: QualityProfileCriteria {
            quality_tiers: vec!["1080P".to_string(), "720P".to_string(), "480P".to_string()],
            archival_quality: Some("1080P".to_string()),
            allow_unknown_quality: false,
            source_allowlist: vec![],
            source_blocklist: vec![],
            video_codec_allowlist: vec![],
            video_codec_blocklist: vec![],
            audio_codec_allowlist: vec![],
            audio_codec_blocklist: vec![],
            atmos_preferred: false,
            dolby_vision_allowed: true,
            detected_hdr_allowed: true,
            prefer_remux: false,
            allow_bd_disk: false,
            allow_upgrades: true,
            prefer_dual_audio: false,
            required_audio_languages: vec![],
            scoring_persona: ScoringPersona::default(),
            scoring_overrides: ScoringOverrides::default(),
            cutoff_tier: Some(cutoff_tier.to_string()),
            min_score_to_grab: None,
            facet_persona_overrides: HashMap::new(),
        },
    }
}

pub(super) fn queue_history_fixture_item(
    download_client_item_id: &str,
    state: DownloadQueueState,
    last_updated_at: i64,
) -> DownloadQueueItem {
    DownloadQueueItem {
        id: download_client_item_id.to_string(),
        title_id: Some("title-1".to_string()),
        episode_id: None,
        title_name: format!("Fixture {download_client_item_id}"),
        facet: Some("movie".to_string()),
        category: None,
        client_id: "primary".to_string(),
        client_name: "Primary".to_string(),
        client_type: "nzbget".to_string(),
        state,
        progress_percent: 100,
        import_transfer_phase: None,
        import_transfer_bytes: None,
        import_transfer_total_bytes: None,
        import_transfer_started_at: None,
        import_transfer_updated_at: None,
        size_bytes: None,
        remaining_seconds: None,
        queued_at: None,
        last_updated_at: Some(last_updated_at.to_string()),
        attention_required: false,
        attention_reason: None,
        download_client_item_id: download_client_item_id.to_string(),
        download_id: None,
        import_status: None,
        import_error_code: None,
        import_error_message: None,
        imported_at: None,
        delete_status: None,
        delete_error_message: None,
        source_provider: None,
        is_scryer_origin: true,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
    }
}

pub(super) fn completed_download_fixture_item(
    download_client_item_id: &str,
    title_id: &str,
    name: &str,
    dest_dir: &str,
) -> CompletedDownload {
    CompletedDownload {
        client_type: "nzbget".to_string(),
        client_id: "primary".to_string(),
        download_client_item_id: download_client_item_id.to_string(),
        download_id: None,
        name: name.to_string(),
        dest_dir: dest_dir.to_string(),
        category: Some("movie".to_string()),
        size_bytes: None,
        completed_at: Some(Utc::now()),
        parameters: vec![
            ("*scryer_title_id".to_string(), title_id.to_string()),
            ("*scryer_facet".to_string(), "movie".to_string()),
        ],
    }
}

pub(super) async fn insert_tracked_download_snapshot(
    app: &AppUseCase,
    item_id: &str,
    state: TrackedDownloadState,
    mut client_item: DownloadQueueItem,
) {
    let tracked_id =
        crate::tracked_downloads::tracked_download_id(Some("primary"), "nzbget", item_id);
    client_item.download_client_item_id = item_id.to_string();
    let title_id = client_item.title_id.clone();
    let facet = client_item.facet.clone();
    let source_title =
        Some(client_item.title_name.clone()).filter(|value| !value.trim().is_empty());
    app.runtime
        .acquisition
        .tracked_download_snapshot
        .write()
        .await
        .insert(
            tracked_id,
            crate::tracked_downloads::TrackedDownloadQueueMetadata {
                client_item,
                client_id: "primary".to_string(),
                client_type: "nzbget".to_string(),
                title_id,
                facet,
                source_title,
                state,
                status: scryer_domain::TrackedDownloadStatus::Warning,
                status_messages: vec![format!("tracked {}", state.as_str())],
                match_type: scryer_domain::TitleMatchType::Submission,
                foreign_import_classification: None,
            },
        );
}

pub(super) async fn create_enabled_download_client_config(
    app: &AppUseCase,
    user: &User,
    name: &str,
    client_type: &str,
) -> DownloadClientConfig {
    app.create_download_client_config(
        user,
        NewDownloadClientConfig {
            name: name.to_string(),
            client_type: client_type.to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config")
}

pub(super) async fn seed_download_client_config(
    app: &AppUseCase,
    id: &str,
    name: &str,
    client_type: &str,
) -> DownloadClientConfig {
    app.services
        .integrations
        .download_client_configs
        .create(DownloadClientConfig {
            id: id.to_string(),
            name: name.to_string(),
            client_type: client_type.to_string(),
            config_json: "{}".to_string(),
            is_enabled: true,
            status: scryer_domain::DownloadClientStatus::Healthy,
            last_error: None,
            last_seen_at: None,
            client_priority: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .expect("seed download client config")
}

pub(super) async fn set_download_client_cleanup_routing(
    app: &AppUseCase,
    user: &User,
    facet: &str,
    client_id: &str,
    remove_completed: bool,
    remove_failed: bool,
) {
    app.update_download_client_routing(
        user,
        facet,
        vec![DownloadClientRoutingSettingsEntry {
            client_id: client_id.to_string(),
            enabled: true,
            category: None,
            recent_queue_priority: None,
            older_queue_priority: None,
            remove_completed,
            remove_failed,
        }],
    )
    .await
    .expect("update download client routing");
}

pub(super) fn failed_history_item(
    download_client_item_id: &str,
    title_name: &str,
) -> DownloadQueueItem {
    DownloadQueueItem {
        id: download_client_item_id.to_string(),
        title_id: None,
        episode_id: None,
        title_name: title_name.to_string(),
        facet: Some("movie".to_string()),
        category: None,
        client_id: "primary".to_string(),
        client_name: "Primary".to_string(),
        client_type: "nzbget".to_string(),
        state: DownloadQueueState::Failed,
        progress_percent: 100,
        import_transfer_phase: None,
        import_transfer_bytes: None,
        import_transfer_total_bytes: None,
        import_transfer_started_at: None,
        import_transfer_updated_at: None,
        size_bytes: None,
        remaining_seconds: None,
        queued_at: None,
        last_updated_at: None,
        attention_required: true,
        attention_reason: Some("corrupt archive".to_string()),
        download_client_item_id: download_client_item_id.to_string(),
        download_id: None,
        import_status: None,
        import_error_code: None,
        import_error_message: None,
        imported_at: None,
        delete_status: None,
        delete_error_message: None,
        source_provider: None,
        is_scryer_origin: true,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
    }
}

pub(super) fn pending_movie_release(
    wanted_id: &str,
    title: &scryer_domain::Title,
    release_title: &str,
    status: PendingReleaseStatus,
) -> PendingRelease {
    let now = Utc::now();
    PendingRelease {
        id: Id::new().0,
        wanted_item_id: wanted_id.to_string(),
        title_id: title.id.clone(),
        release_title: release_title.to_string(),
        release_url: Some(format!(
            "https://example.invalid/{}.nzb",
            release_title.to_ascii_lowercase().replace(' ', ".")
        )),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        release_size_bytes: Some(1_000),
        release_score: 1000,
        scoring_log_json: None,
        indexer_source: Some("test-indexer".to_string()),
        release_guid: Some(format!("{release_title}-guid")),
        added_at: (now - chrono::Duration::minutes(5)).to_rfc3339(),
        delay_until: (now - chrono::Duration::minutes(1)).to_rfc3339(),
        status,
        grabbed_at: None,
        source_password: None,
        published_at: Some(now.to_rfc3339()),
        info_hash: None,
    }
}

pub(super) async fn seed_movie_wanted_for_acquisition(
    app: &AppUseCase,
    user: &User,
    acquisition_scope_states: &Arc<TrackingAcquisitionScopeStateRepo>,
    name: &str,
    year: i32,
) -> (scryer_domain::Title, String) {
    let title = app
        .add_title(
            user,
            NewTitle {
                name: name.to_string(),
                sort_title: Some(name.to_string()),
                slug: Some(name.to_ascii_lowercase().replace(' ', "-")),
                facet: MediaFacet::Movie,
                monitored: true,
                year: Some(year),
                content_status: Some("Released".to_string()),
                min_availability: Some("released".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("create movie title");
    acquisition_scope_states
        .remember_title_facet(&title.id, MediaFacet::Movie)
        .await;
    let wanted_id = Id::new().0;
    acquisition_scope_states
        .upsert_acquisition_scope_state(&AcquisitionScopeState {
            id: wanted_id.clone(),
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_slug: title.slug.clone(),
            title_facet: Some(MediaFacet::Movie.as_str().to_string()),
            library_id: Some(title.library_id.clone()),
            library_name: Some("Movies".to_string()),
            library_slug: Some("movies".to_string()),
            episode_id: None,
            collection_id: None,
            series_movie_link_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            last_search_at: None,
            status: AcquisitionScopeStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed movie wanted item");

    (title, wanted_id)
}

pub(super) async fn seed_anime_season_wanted_for_acquisition(
    app: &AppUseCase,
    user: &User,
    acquisition_scope_states: &Arc<TrackingAcquisitionScopeStateRepo>,
    name: &str,
    season_number: u32,
) -> (scryer_domain::Title, Vec<String>) {
    let title = app
        .add_title(
            user,
            NewTitle {
                name: name.to_string(),
                sort_title: Some(name.to_string()),
                slug: Some(name.to_ascii_lowercase().replace(' ', "-")),
                facet: MediaFacet::Anime,
                monitored: true,
                runtime_minutes: Some(24),
                content_status: Some("Continuing".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");
    acquisition_scope_states
        .remember_title_facet(&title.id, MediaFacet::Anime)
        .await;
    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: season_number.to_string(),
            label: Some(format!("Season {season_number}")),
            ordered_path: None,
            narrative_order: Some(season_number.to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("2".to_string()),
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");

    let mut wanted_ids = Vec::new();
    for episode_number in 1..=2 {
        let episode_label = format!("S{season_number:02}E{episode_number:02}");
        let episode = app
            .services
            .catalog
            .shows
            .create_episode(Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id: Some(season.id.clone()),
                episode_type: scryer_domain::EpisodeType::Standard,
                episode_number: Some(episode_number.to_string()),
                season_number: Some(season_number.to_string()),
                episode_label: Some(episode_label.clone()),
                title: Some(episode_label),
                air_date: Some("2024-01-01".to_string()),
                duration_seconds: Some(1_440),
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: false,
                is_recap: false,
                absolute_number: None,
                overview: None,
                tvdb_id: None,
                image_url: None,
                monitored: true,
                created_at: Utc::now(),
            })
            .await
            .expect("create episode");
        let wanted_id = Id::new().0;
        wanted_ids.push(wanted_id.clone());
        acquisition_scope_states
            .upsert_acquisition_scope_state(&AcquisitionScopeState {
                id: wanted_id,
                title_id: title.id.clone(),
                title_name: Some(title.name.clone()),
                title_slug: title.slug.clone(),
                title_facet: Some(MediaFacet::Anime.as_str().to_string()),
                library_id: Some(title.library_id.clone()),
                library_name: Some("Anime".to_string()),
                library_slug: Some("anime".to_string()),
                episode_id: Some(episode.id.clone()),
                collection_id: Some(season.id.clone()),
                series_movie_link_id: None,
                season_number: Some(season_number.to_string()),
                episode_number: Some(episode_number.to_string()),
                media_type: "episode".to_string(),
                last_search_at: None,
                status: AcquisitionScopeStatus::Wanted,
                grabbed_release: None,
                current_score: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            })
            .await
            .expect("seed episode wanted item");
    }

    (title, wanted_ids)
}

pub(super) async fn create_series_with_collection_and_episode(
    app: &AppUseCase,
    user: &User,
    name: &str,
) -> (Title, Collection, Episode) {
    let title = app
        .add_title(
            user,
            NewTitle {
                name: name.into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let collection = app
        .create_collection(
            user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");

    let episode = app
        .create_episode(
            user,
            title.id.clone(),
            Some(collection.id.clone()),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("Pilot".into()),
            Some("Pilot".into()),
            None,
            Some(1_200),
            false,
            false,
        )
        .await
        .expect("create episode");

    (title, collection, episode)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TestPermissionPreset {
    CatalogView,
    MediaRequest,
    TitleManagement,
    UserManagement,
    ConfigManagement,
}

/// Derive a per-user JWT signing key (mirrors `AppUseCase::derive_jwt_key`).
pub(super) fn test_derive_jwt_key(
    salt: &str,
    password_hash: &str,
    permissions: &[TestPermissionPreset],
) -> Vec<u8> {
    use aws_lc_rs::hmac;
    let app_permissions = test_app_permissions_from_presets(permissions);
    let library_grants = test_library_grants_from_presets(permissions);
    let mut app_claims = app_permissions
        .to_permissions()
        .into_iter()
        .map(AppUseCase::app_permission_claim_string)
        .map(str::to_string)
        .collect::<Vec<_>>();
    app_claims.sort();
    app_claims.dedup();
    let mut library_claims = library_grants
        .into_iter()
        .map(|grant| {
            let mut permissions = grant
                .permissions
                .to_permissions()
                .into_iter()
                .map(AppUseCase::library_permission_claim_string)
                .map(str::to_string)
                .collect::<Vec<_>>();
            permissions.sort();
            permissions.dedup();
            format!("{}:{}", grant.library_id, permissions.join(","))
        })
        .collect::<Vec<_>>();
    library_claims.sort();
    let authorization_fingerprint = sha256_hex(format!(
        "app\n{}\nlibrary\n{}",
        app_claims.join("\n"),
        library_claims.join("\n")
    ));
    let signing_material = format!("{password_hash}\n{authorization_fingerprint}");
    let hmac_key = hmac::Key::new(hmac::HMAC_SHA256, salt.as_bytes());
    hmac::sign(&hmac_key, signing_material.as_bytes())
        .as_ref()
        .to_vec()
}

pub(super) const TEST_PASSWORD_HASH: &str =
    "v2$argon2id$v=19$m=19456,t=2,p=1$dGVzdHNhbHQ$dGVzdGhhc2g";

pub(super) fn test_app_permissions_from_presets(
    permissions: &[TestPermissionPreset],
) -> scryer_domain::AppPermissionMask {
    let mut mask = scryer_domain::AppPermissionMask::NONE;
    if permissions.contains(&TestPermissionPreset::UserManagement) {
        mask.insert(scryer_domain::AppPermissionMask::MANAGE_USERS);
        mask.insert(scryer_domain::AppPermissionMask::MANAGE_PERMISSIONS);
    }
    if permissions.contains(&TestPermissionPreset::ConfigManagement) {
        mask.insert(scryer_domain::AppPermissionMask::MANAGE_SYSTEM_SETTINGS);
        mask.insert(scryer_domain::AppPermissionMask::MANAGE_CATALOG_SETTINGS);
    }
    mask
}

pub(super) fn test_library_grants_from_presets(
    presets: &[TestPermissionPreset],
) -> Vec<scryer_domain::LibraryGrant> {
    let mut permissions = scryer_domain::LibraryPermissionMask::NONE;
    if presets.contains(&TestPermissionPreset::CatalogView) {
        permissions.insert(scryer_domain::LibraryPermissionMask::VIEW);
    }
    if presets.contains(&TestPermissionPreset::MediaRequest) {
        permissions.insert(scryer_domain::LibraryPermissionMask::REQUEST);
    }
    if presets.contains(&TestPermissionPreset::TitleManagement) {
        permissions.insert(scryer_domain::LibraryPermissionMask::VIEW);
        permissions.insert(scryer_domain::LibraryPermissionMask::MANAGE_TITLES);
        permissions.insert(scryer_domain::LibraryPermissionMask::RESOLVE_IMPORTS);
        permissions.insert(scryer_domain::LibraryPermissionMask::MANAGE_LIBRARY);
    }
    if permissions.is_empty() {
        return Vec::new();
    }
    [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime]
        .into_iter()
        .map(|facet| scryer_domain::LibraryGrant {
            user_id: String::new(),
            library_id: scryer_domain::default_library_id_for_facet(&facet),
            permissions,
        })
        .collect()
}

pub(super) async fn create_user_with_permissions(
    app: &AppUseCase,
    actor: &User,
    username: &str,
    password: &str,
    permissions: Vec<TestPermissionPreset>,
) -> AppResult<User> {
    app.create_user(
        actor,
        username.to_string(),
        password.to_string(),
        test_app_permissions_from_presets(&permissions),
        test_library_grants_from_presets(&permissions),
    )
    .await
}

pub(super) async fn create_authenticated_user(
    app: &AppUseCase,
    admin: &User,
    username: &str,
    password: &str,
    permissions: Vec<TestPermissionPreset>,
) -> (User, User) {
    let created = create_user_with_permissions(app, admin, username, password, permissions)
        .await
        .expect("create user");
    let token = app.issue_access_token(&created).await.expect("issue token");
    let authenticated = app
        .authenticate_token(&token)
        .await
        .expect("authenticate token");

    (created, authenticated)
}
