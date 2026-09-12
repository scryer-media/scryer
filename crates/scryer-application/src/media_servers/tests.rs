use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tokio::sync::Mutex;

use super::*;
use crate::null_repositories::NullSettingsRepository;
use crate::null_repositories::test_nulls::{
    NullDownloadClient, NullDownloadClientConfigRepository, NullIndexerClient,
    NullQualityProfileRepository, NullReleaseAttemptRepository, NullShowRepository,
    NullTitleRepository, NullUserRepository,
};
use crate::services::AppServices;
use scryer_domain::{
    LibraryPermission, LibraryPermissionMask, MediaServerPlaybackEntityKind,
    MediaServerPlaybackItem, UserAuthorization,
};

#[derive(Default)]
struct TestMediaServerConnectionRepository {
    connections: Mutex<Vec<MediaServerConnection>>,
    playback_items: Mutex<Vec<MediaServerPlaybackItem>>,
    fail_create: bool,
    fail_update: bool,
}

impl TestMediaServerConnectionRepository {
    fn new(connections: Vec<MediaServerConnection>) -> Self {
        Self {
            connections: Mutex::new(connections),
            playback_items: Mutex::new(Vec::new()),
            fail_create: false,
            fail_update: false,
        }
    }

    fn failing(connections: Vec<MediaServerConnection>, create: bool, update: bool) -> Self {
        Self {
            connections: Mutex::new(connections),
            playback_items: Mutex::new(Vec::new()),
            fail_create: create,
            fail_update: update,
        }
    }
}

#[async_trait::async_trait]
impl MediaServerConnectionRepository for TestMediaServerConnectionRepository {
    async fn list(
        &self,
        provider: Option<MediaServerProvider>,
    ) -> AppResult<Vec<MediaServerConnection>> {
        Ok(self
            .connections
            .lock()
            .await
            .iter()
            .filter(|connection| {
                provider
                    .as_ref()
                    .is_none_or(|provider| &connection.provider == provider)
            })
            .cloned()
            .collect())
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<MediaServerConnection>> {
        Ok(self
            .connections
            .lock()
            .await
            .iter()
            .find(|connection| connection.id == id)
            .cloned())
    }

    async fn create(&self, connection: MediaServerConnection) -> AppResult<MediaServerConnection> {
        if self.fail_create {
            return Err(AppError::Repository("injected create failure".into()));
        }
        self.connections.lock().await.push(connection.clone());
        Ok(connection)
    }

    async fn update(&self, connection: MediaServerConnection) -> AppResult<MediaServerConnection> {
        if self.fail_update {
            return Err(AppError::Repository("injected update failure".into()));
        }
        let mut connections = self.connections.lock().await;
        if let Some(existing) = connections
            .iter_mut()
            .find(|candidate| candidate.id == connection.id)
        {
            *existing = connection.clone();
        }
        Ok(connection)
    }

    async fn list_playback_items_for_entity(
        &self,
        entity_kind: MediaServerPlaybackEntityKind,
        entity_id: &str,
    ) -> AppResult<Vec<MediaServerPlaybackItem>> {
        Ok(self
            .playback_items
            .lock()
            .await
            .iter()
            .filter(|item| item.entity_kind == entity_kind && item.entity_id == entity_id)
            .cloned()
            .collect())
    }

    async fn list_playback_items_for_entities(
        &self,
        entities: &[(MediaServerPlaybackEntityKind, String)],
    ) -> AppResult<Vec<MediaServerPlaybackItem>> {
        Ok(self
            .playback_items
            .lock()
            .await
            .iter()
            .filter(|item| {
                entities.iter().any(|(entity_kind, entity_id)| {
                    item.entity_kind == *entity_kind && item.entity_id == *entity_id
                })
            })
            .cloned()
            .collect())
    }

    async fn upsert_playback_items_for_connection(
        &self,
        connection_id: &str,
        items: Vec<MediaServerPlaybackItem>,
    ) -> AppResult<()> {
        if items.iter().any(|item| item.connection_id != connection_id) {
            return Err(AppError::Validation(
                "playback mapping connection ID does not match upsert target".into(),
            ));
        }
        let mut playback_items = self.playback_items.lock().await;
        for item in items {
            playback_items.retain(|existing| {
                !(existing.connection_id == item.connection_id
                    && existing.entity_kind == item.entity_kind
                    && existing.entity_id == item.entity_id)
            });
            playback_items.push(item);
        }
        Ok(())
    }

    async fn replace_playback_items_for_connection(
        &self,
        connection_id: &str,
        items: Vec<MediaServerPlaybackItem>,
    ) -> AppResult<()> {
        if items.iter().any(|item| item.connection_id != connection_id) {
            return Err(AppError::Validation(
                "playback mapping connection ID does not match replacement target".into(),
            ));
        }
        let mut playback_items = self.playback_items.lock().await;
        playback_items.retain(|item| item.connection_id != connection_id);
        playback_items.extend(items);
        Ok(())
    }

    async fn compare_and_set_emby_base_url(
        &self,
        connection_id: &str,
        expected_base_url: &str,
        expected_server_id: &str,
        new_base_url: &str,
    ) -> AppResult<bool> {
        let mut connections = self.connections.lock().await;
        let Some(connection) = connections.iter_mut().find(|connection| {
            connection.id == connection_id
                && connection.base_url == expected_base_url
                && connection.emby_server_id.as_deref() == Some(expected_server_id)
        }) else {
            return Ok(false);
        };
        connection.base_url = new_base_url.to_string();
        Ok(true)
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.connections
            .lock()
            .await
            .retain(|connection| connection.id != id);
        Ok(())
    }

    async fn has_external_accounts(&self, _: &str) -> AppResult<bool> {
        Ok(false)
    }

    async fn has_notification_channels(&self, _: &str) -> AppResult<bool> {
        Ok(false)
    }
}

struct TestIndexerConfigRepository;

#[async_trait::async_trait]
impl IndexerConfigRepository for TestIndexerConfigRepository {
    async fn list(&self, _: Option<String>) -> AppResult<Vec<IndexerConfig>> {
        Ok(Vec::new())
    }

    async fn get_by_id(&self, _: &str) -> AppResult<Option<IndexerConfig>> {
        Ok(None)
    }

    async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
        Ok(config)
    }

    async fn touch_last_error(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn update(&self, _: IndexerConfigUpdate) -> AppResult<IndexerConfig> {
        Err(AppError::Repository(
            "indexer config update is not configured".into(),
        ))
    }

    async fn delete(&self, _: &str) -> AppResult<()> {
        Ok(())
    }
}

struct NoopExternalIdentityVerifier;

#[async_trait::async_trait]
impl ExternalIdentityVerifier for NoopExternalIdentityVerifier {
    async fn verify_plex(
        &self,
        _: &str,
        _: Option<&str>,
        _: &str,
    ) -> AppResult<VerifiedExternalIdentity> {
        Err(AppError::Repository(
            "plex verification is not configured".into(),
        ))
    }

    async fn discover_plex_servers(&self, _: &str) -> AppResult<Vec<PlexServerDiscovery>> {
        Ok(vec![PlexServerDiscovery {
            id: "machine-2".to_string(),
            name: "Plex 2".to_string(),
        }])
    }

    async fn verify_jellyfin(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
    ) -> AppResult<VerifiedExternalIdentity> {
        Err(AppError::Repository(
            "jellyfin verification is not configured".into(),
        ))
    }

    async fn test_jellyfin_connection(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn test_jellyfin_api_key(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn exchange_jellyfin_admin_api_key(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
    ) -> AppResult<String> {
        Ok("generated-api-key".to_string())
    }

    async fn list_jellyfin_users(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> AppResult<Vec<JellyfinServerUser>> {
        Ok(Vec::new())
    }

    async fn list_plex_users(&self, _: &str, _: Option<&str>) -> AppResult<Vec<PlexServerUser>> {
        Ok(Vec::new())
    }
}

struct CountingExternalIdentityVerifier {
    test_jellyfin_api_key_calls: Arc<AtomicUsize>,
    emby_avatar_fetch_calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl ExternalIdentityVerifier for CountingExternalIdentityVerifier {
    async fn verify_plex(
        &self,
        _: &str,
        _: Option<&str>,
        _: &str,
    ) -> AppResult<VerifiedExternalIdentity> {
        Err(AppError::Repository(
            "plex verification is not configured".into(),
        ))
    }

    async fn discover_plex_servers(&self, _: &str) -> AppResult<Vec<PlexServerDiscovery>> {
        Ok(Vec::new())
    }

    async fn verify_jellyfin(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
    ) -> AppResult<VerifiedExternalIdentity> {
        Err(AppError::Repository(
            "jellyfin verification is not configured".into(),
        ))
    }

    async fn test_jellyfin_connection(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn test_jellyfin_api_key(&self, _: &str, _: &str) -> AppResult<()> {
        self.test_jellyfin_api_key_calls
            .fetch_add(1, Ordering::SeqCst);
        Err(AppError::Repository("Jellyfin is unreachable".into()))
    }

    async fn exchange_jellyfin_admin_api_key(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
    ) -> AppResult<String> {
        Ok("generated-api-key".to_string())
    }

    async fn list_jellyfin_users(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> AppResult<Vec<JellyfinServerUser>> {
        Ok(Vec::new())
    }

    async fn list_plex_users(&self, _: &str, _: Option<&str>) -> AppResult<Vec<PlexServerUser>> {
        Ok(Vec::new())
    }

    async fn fetch_emby_user_avatar(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
    ) -> AppResult<Option<EmbyAvatar>> {
        self.emby_avatar_fetch_calls.fetch_add(1, Ordering::SeqCst);
        Ok(Some(EmbyAvatar {
            content_type: "image/png".into(),
            bytes: vec![1, 2, 3],
            etag: None,
            last_modified: None,
        }))
    }
}

struct EmbySetupVerifier {
    finish_compensation: Arc<Mutex<Vec<bool>>>,
    local_admin_passwords: Arc<Mutex<Vec<String>>>,
    connect_passwords: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl ExternalIdentityVerifier for EmbySetupVerifier {
    async fn verify_plex(
        &self,
        _: &str,
        _: Option<&str>,
        _: &str,
    ) -> AppResult<VerifiedExternalIdentity> {
        unreachable!()
    }

    async fn discover_plex_servers(&self, _: &str) -> AppResult<Vec<PlexServerDiscovery>> {
        Ok(Vec::new())
    }

    async fn verify_jellyfin(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
    ) -> AppResult<VerifiedExternalIdentity> {
        unreachable!()
    }

    async fn test_jellyfin_connection(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn test_jellyfin_api_key(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn exchange_jellyfin_admin_api_key(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
    ) -> AppResult<String> {
        unreachable!()
    }

    async fn list_jellyfin_users(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> AppResult<Vec<JellyfinServerUser>> {
        Ok(Vec::new())
    }

    async fn exchange_emby_local_admin_api_key(
        &self,
        _: &str,
        _: &str,
        _: &str,
        password: &str,
    ) -> AppResult<EmbyApiKeyExchange> {
        self.local_admin_passwords
            .lock()
            .await
            .push(password.to_string());
        Ok(EmbyApiKeyExchange {
            api_key: "new-key".into(),
            server_identity: EmbyServerIdentity {
                api_base_url: "https://emby.example.test".into(),
                server_id: "emby-server-id".into(),
                server_name: "Emby".into(),
                version: "4.9.5.0".into(),
            },
            created_new_key: true,
            cleanup: Some(EmbyApiKeyExchangeCleanup::new(
                "https://emby.example.test".into(),
                "admin-id".into(),
                "temporary-token".into(),
                Some("new-key".into()),
            )),
        })
    }

    async fn exchange_emby_connect_admin_api_key(
        &self,
        _: &str,
        _: &str,
        server_id: &str,
        _: &str,
        password: &str,
    ) -> AppResult<EmbyApiKeyExchange> {
        self.connect_passwords
            .lock()
            .await
            .push(password.to_string());
        Ok(EmbyApiKeyExchange {
            api_key: "connect-key".into(),
            server_identity: EmbyServerIdentity {
                api_base_url: "https://emby.example.test/emby".into(),
                server_id: server_id.into(),
                server_name: "Emby".into(),
                version: "4.9.5.0".into(),
            },
            created_new_key: false,
            cleanup: None,
        })
    }

    async fn test_emby_api_key(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> AppResult<EmbyServerIdentity> {
        Ok(EmbyServerIdentity {
            api_base_url: "https://emby.example.test".into(),
            server_id: "emby-server-id".into(),
            server_name: "Emby".into(),
            version: "4.9.5.0".into(),
        })
    }

    async fn finish_emby_api_key_exchange(
        &self,
        _: &str,
        _: EmbyApiKeyExchangeCleanup,
        compensate_created_key: bool,
    ) {
        self.finish_compensation
            .lock()
            .await
            .push(compensate_created_key);
    }

    async fn list_plex_users(&self, _: &str, _: Option<&str>) -> AppResult<Vec<PlexServerUser>> {
        Ok(Vec::new())
    }
}

struct TestNotificationPluginProvider;

impl NotificationPluginProvider for TestNotificationPluginProvider {
    fn client_for_channel(
        &self,
        _: &scryer_domain::NotificationChannelConfig,
    ) -> Option<Arc<dyn NotificationClient>> {
        None
    }

    fn available_provider_types(&self) -> Vec<String> {
        vec!["plex".to_string()]
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        if provider_type != "plex" {
            return Vec::new();
        }

        vec![scryer_domain::ConfigFieldDef {
            key: "base_url".to_string(),
            label: "Base URL".to_string(),
            field_type: scryer_domain::ConfigFieldType::String,
            required: true,
            default_value: None,
            value_source: Default::default(),
            role: None,
            host_binding: None,
            options: Vec::new(),
            help_text: None,
            ..Default::default()
        }]
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        (provider_type == "plex").then(|| "Plex Media Server".to_string())
    }
}

struct TestDomainEventRepository;

#[async_trait::async_trait]
impl DomainEventRepository for TestDomainEventRepository {
    async fn append(&self, event: NewDomainEvent) -> AppResult<DomainEvent> {
        Ok(DomainEvent {
            sequence: 1,
            event_id: event.event_id,
            occurred_at: event.occurred_at,
            actor_kind: event.actor_kind,
            actor_user_id: event.actor_user_id,
            actor_display_name: event.actor_display_name,
            title_id: event.title_id,
            facet: event.facet,
            correlation_id: event.correlation_id,
            causation_id: event.causation_id,
            schema_version: event.schema_version,
            stream: event.stream,
            payload: event.payload,
        })
    }

    async fn append_many(&self, events: Vec<NewDomainEvent>) -> AppResult<Vec<DomainEvent>> {
        let mut appended = Vec::new();
        for event in events {
            appended.push(self.append(event).await?);
        }
        Ok(appended)
    }

    async fn list(&self, _: &DomainEventFilter) -> AppResult<Vec<DomainEvent>> {
        Ok(Vec::new())
    }

    async fn count_title_history_page_events(
        &self,
        _: Option<&[TitleHistoryEventType]>,
        _: Option<&[String]>,
        _: Option<&str>,
    ) -> AppResult<i64> {
        Ok(0)
    }

    async fn count_dashboard_activity_events(
        &self,
        _: &[String],
        _: chrono::DateTime<chrono::Utc>,
        _: chrono::DateTime<chrono::Utc>,
        _: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<crate::DashboardActivityStats> {
        Ok(crate::DashboardActivityStats::default())
    }

    async fn list_title_history_page_events(
        &self,
        _: Option<&[TitleHistoryEventType]>,
        _: Option<&[String]>,
        _: Option<&str>,
        _: usize,
        _: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        Ok(Vec::new())
    }

    async fn list_after_sequence(&self, _: i64, _: usize) -> AppResult<Vec<DomainEvent>> {
        Ok(Vec::new())
    }

    async fn delete_for_title_ids(&self, _: &[String]) -> AppResult<u32> {
        Ok(0)
    }

    async fn get_subscriber_offset(&self, _: &str) -> AppResult<i64> {
        Ok(0)
    }

    async fn set_subscriber_offset(&self, _: &str, _: i64) -> AppResult<()> {
        Ok(())
    }
}

fn app_with_connections_and_verifier(
    connections: Vec<MediaServerConnection>,
    verifier: Arc<dyn ExternalIdentityVerifier>,
) -> AppUseCase {
    app_with_repository_and_verifier(
        Arc::new(TestMediaServerConnectionRepository::new(connections)),
        verifier,
    )
}

fn app_with_repository_and_verifier(
    repository: Arc<dyn MediaServerConnectionRepository>,
    verifier: Arc<dyn ExternalIdentityVerifier>,
) -> AppUseCase {
    let services = AppServices::builder(
        Arc::new(NullTitleRepository),
        Arc::new(NullShowRepository),
        Arc::new(NullUserRepository),
        Arc::new(TestIndexerConfigRepository),
        Arc::new(NullIndexerClient),
        Arc::new(NullDownloadClient),
        Arc::new(NullDownloadClientConfigRepository),
        Arc::new(NullReleaseAttemptRepository),
        Arc::new(NullSettingsRepository),
        Arc::new(NullQualityProfileRepository),
        String::new(),
    )
    .with_external_identity_verifier(verifier)
    .with_media_server_connection_store(repository)
    .with_notification_provider(Arc::new(TestNotificationPluginProvider))
    .with_domain_events(Arc::new(TestDomainEventRepository))
    .build_partial_for_tests();

    AppUseCase::new(
        services,
        JwtAuthConfig {
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(FacetRegistry::new()),
    )
}

fn app_with_connections(connections: Vec<MediaServerConnection>) -> AppUseCase {
    app_with_connections_and_verifier(connections, Arc::new(NoopExternalIdentityVerifier))
}

fn app_with_connection(connection: MediaServerConnection) -> AppUseCase {
    app_with_connections(vec![connection])
}

fn user_with_permissions(username: &str, app: AppPermissionMask) -> User {
    User {
        id: username.to_string(),
        username: username.to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: UserAuthorization {
            app,
            libraries: HashMap::new(),
            default_library: LibraryPermissionMask::NONE,
            actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            login_status: Default::default(),
            loaded: true,
        },
    }
}

fn system_settings_user() -> User {
    user_with_permissions(
        "system-settings",
        AppPermissionMask::from_permissions([AppPermission::ManageSystemSettings]),
    )
}

fn permission_manager_user() -> User {
    user_with_permissions(
        "permission-manager",
        AppPermissionMask::from_permissions([
            AppPermission::ManageSystemSettings,
            AppPermission::ManagePermissions,
        ]),
    )
}

fn grant_bearing_jellyfin_connection() -> MediaServerConnection {
    let now = Utc::now();
    MediaServerConnection {
        id: "jellyfin-main".to_string(),
        provider: MediaServerProvider::Jellyfin,
        display_name: "Jellyfin".to_string(),
        base_url: "https://jellyfin.example.test".to_string(),
        external_url: None,
        enabled: true,
        login_enabled: true,
        linking_enabled: false,
        auto_add_enabled: true,
        default_app_permissions: AppPermissionMask::from_permissions([
            AppPermission::ManageCatalogSettings,
        ]),
        default_library_grants: vec![MediaServerDefaultLibraryGrant {
            library_id: "movies".to_string(),
            permissions: LibraryPermissionMask::from_permissions([LibraryPermission::View]),
        }],
        machine_id: None,
        api_key: Some("api-key-1".to_string()),
        emby_server_id: None,
        emby_connect_enabled: false,
        path_mappings: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

fn grant_bearing_plex_connection() -> MediaServerConnection {
    let mut connection = grant_bearing_jellyfin_connection();
    connection.id = "plex-main".to_string();
    connection.provider = MediaServerProvider::Plex;
    connection.display_name = "Plex".to_string();
    connection.base_url = "https://plex.tv".to_string();
    connection.machine_id = Some("machine-1".to_string());
    connection.api_key = None;
    connection
}

fn emby_connection(enabled: bool, api_key: Option<&str>) -> MediaServerConnection {
    let mut connection = grant_bearing_jellyfin_connection();
    connection.id = "emby-main".to_string();
    connection.provider = MediaServerProvider::Emby;
    connection.display_name = "Emby".to_string();
    connection.base_url = "https://emby.example.test".to_string();
    connection.enabled = enabled;
    connection.api_key = api_key.map(ToString::to_string);
    connection.emby_server_id = Some("emby-server-id".to_string());
    connection.emby_connect_enabled = false;
    connection
}

fn empty_update_patch(id: &str) -> MediaServerConnectionPatch {
    MediaServerConnectionPatch {
        id: id.to_string(),
        ..Default::default()
    }
}

fn assert_unauthorized(error: AppError) {
    assert!(
        matches!(error, AppError::Unauthorized(_)),
        "expected unauthorized error, got {error:?}",
    );
}

#[tokio::test]
async fn jellyfin_user_listing_without_api_key_points_to_picker_setup() {
    let mut connection = grant_bearing_jellyfin_connection();
    connection.api_key = None;
    let app = app_with_connection(connection);
    let user = user_with_permissions(
        "manage-users",
        AppPermissionMask::from_permissions([AppPermission::ManageUsers]),
    );

    let error = app
        .list_jellyfin_server_users(&user, "jellyfin-main", None)
        .await
        .expect_err("missing Jellyfin API key should fail");
    let message = error.to_string();

    assert!(message.contains("save an API key to load Jellyfin users"));
    assert!(!message.contains("manually"));
}

#[tokio::test]
async fn emby_avatar_fetch_requires_manage_users_before_upstream() {
    let avatar_fetch_calls = Arc::new(AtomicUsize::new(0));
    let app = app_with_connections_and_verifier(
        vec![emby_connection(true, Some("emby-admin-key"))],
        Arc::new(CountingExternalIdentityVerifier {
            test_jellyfin_api_key_calls: Arc::new(AtomicUsize::new(0)),
            emby_avatar_fetch_calls: Arc::clone(&avatar_fetch_calls),
        }),
    );

    let error = app
        .fetch_emby_server_user_avatar(
            &system_settings_user(),
            "emby-main",
            "external-user",
            "avatar-tag",
        )
        .await
        .expect_err("an actor without ManageUsers must not retrieve an Emby avatar");
    assert_unauthorized(error);
    assert_eq!(avatar_fetch_calls.load(Ordering::SeqCst), 0);

    let avatar = app
        .fetch_emby_server_user_avatar(
            &user_with_permissions(
                "manage-users",
                AppPermissionMask::from_permissions([AppPermission::ManageUsers]),
            ),
            "emby-main",
            "external-user",
            "avatar-tag",
        )
        .await
        .expect("ManageUsers actor should retrieve the Emby avatar")
        .expect("configured Emby avatar");
    assert_eq!(avatar.bytes, vec![1, 2, 3]);
    assert_eq!(avatar_fetch_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn media_server_create_with_api_key_tests_connection_on_save() {
    let test_calls = Arc::new(AtomicUsize::new(0));
    let app = app_with_connections_and_verifier(
        Vec::new(),
        Arc::new(CountingExternalIdentityVerifier {
            test_jellyfin_api_key_calls: Arc::clone(&test_calls),
            emby_avatar_fetch_calls: Arc::new(AtomicUsize::new(0)),
        }),
    );

    let error = app
        .create_media_server_connection(
            &system_settings_user(),
            MediaServerConnectionDraft {
                provider: MediaServerProvider::Jellyfin,
                display_name: "Dead Jellyfin".to_string(),
                base_url: "http://127.0.0.1:9".to_string(),
                external_url: None,
                enabled: true,
                login_enabled: false,
                linking_enabled: false,
                auto_add_enabled: false,
                default_app_permissions: AppPermissionMask::NONE,
                default_library_grants: Vec::new(),
                machine_id: None,
                plex_auth_token: None,
                plex_server_id: None,
                api_key: Some("saved-api-key".to_string()),
                admin_username: None,
                admin_password: None,
                emby_connection_mode: None,
                emby_local_setup_method: None,
                emby_connect_enabled: None,
                emby_connect_username_or_email: None,
                emby_connect_password: None,
                emby_connect_server_id: None,
                path_mappings: Vec::new(),
            },
        )
        .await
        .expect_err("API-key media server save should test reachability");

    assert!(error.to_string().contains("Jellyfin is unreachable"));
    assert_eq!(test_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn disabled_emby_connection_can_clear_api_key_without_retesting_it() {
    let app = app_with_connection(emby_connection(true, Some("stored-key")));
    let mut patch = empty_update_patch("emby-main");
    patch.enabled = Some(false);
    patch.clear_api_key = true;

    let updated = app
        .update_media_server_connection(&system_settings_user(), patch)
        .await
        .expect("disabled Emby connection may clear its key");

    assert!(!updated.enabled);
    assert_eq!(updated.api_key, None);
}

#[tokio::test]
async fn enabled_emby_connection_cannot_clear_api_key() {
    let app = app_with_connection(emby_connection(true, Some("stored-key")));
    let mut patch = empty_update_patch("emby-main");
    patch.clear_api_key = true;

    let error = app
        .update_media_server_connection(&permission_manager_user(), patch)
        .await
        .expect_err("enabled Emby connection must retain its key");

    assert!(matches!(error, AppError::Validation(message) if message.contains("retain")));
}

#[tokio::test]
async fn emby_base_url_refresh_compare_and_set_never_clobbers_concurrent_changes() {
    let repository =
        TestMediaServerConnectionRepository::new(vec![emby_connection(true, Some("stored-key"))]);

    assert!(
        !repository
            .compare_and_set_emby_base_url(
                "emby-main",
                "https://stale.example.test",
                "emby-server-id",
                "https://fresh.example.test",
            )
            .await
            .expect("stale CAS")
    );
    assert!(
        !repository
            .compare_and_set_emby_base_url(
                "emby-main",
                "https://emby.example.test",
                "different-server",
                "https://fresh.example.test",
            )
            .await
            .expect("server mismatch CAS")
    );
    assert_eq!(
        repository
            .get_by_id("emby-main")
            .await
            .expect("read connection")
            .expect("connection")
            .base_url,
        "https://emby.example.test"
    );
    assert!(
        repository
            .compare_and_set_emby_base_url(
                "emby-main",
                "https://emby.example.test",
                "emby-server-id",
                "https://fresh.example.test",
            )
            .await
            .expect("matching CAS")
    );
}

#[tokio::test]
async fn emby_setup_preserves_admin_and_connect_password_bytes() {
    let local_passwords = Arc::new(Mutex::new(Vec::new()));
    let connect_passwords = Arc::new(Mutex::new(Vec::new()));
    let verifier = Arc::new(EmbySetupVerifier {
        finish_compensation: Arc::new(Mutex::new(Vec::new())),
        local_admin_passwords: Arc::clone(&local_passwords),
        connect_passwords: Arc::clone(&connect_passwords),
    });
    let app = app_with_repository_and_verifier(
        Arc::new(TestMediaServerConnectionRepository::new(Vec::new())),
        verifier,
    );
    let draft = |mode, local_password: Option<&str>, connect_password: Option<&str>| {
        MediaServerConnectionDraft {
            provider: MediaServerProvider::Emby,
            display_name: "Emby".into(),
            base_url: "https://emby.example.test".into(),
            external_url: None,
            enabled: true,
            login_enabled: false,
            linking_enabled: false,
            auto_add_enabled: false,
            default_app_permissions: AppPermissionMask::NONE,
            default_library_grants: Vec::new(),
            machine_id: None,
            plex_auth_token: None,
            plex_server_id: None,
            api_key: None,
            admin_username: (mode == EmbyConnectionMode::Local).then(|| " admin ".into()),
            admin_password: local_password.map(str::to_string),
            emby_connection_mode: Some(mode),
            emby_local_setup_method: Some(EmbyLocalSetupMethod::AdminCredentials),
            emby_connect_enabled: Some(mode == EmbyConnectionMode::Connect),
            emby_connect_username_or_email: (mode == EmbyConnectionMode::Connect)
                .then(|| " connect@example.test ".into()),
            emby_connect_password: connect_password.map(str::to_string),
            emby_connect_server_id: (mode == EmbyConnectionMode::Connect)
                .then(|| "emby-server-id".into()),
            path_mappings: Vec::new(),
        }
    };

    let empty_local = app
        .create_media_server_connection(
            &system_settings_user(),
            draft(EmbyConnectionMode::Local, Some(""), None),
        )
        .await;
    assert!(
        matches!(empty_local, Err(AppError::Validation(message)) if message == "both Emby administrator username and password are required")
    );
    let empty_connect = app
        .create_media_server_connection(
            &system_settings_user(),
            draft(EmbyConnectionMode::Connect, None, Some("")),
        )
        .await;
    assert!(
        matches!(empty_connect, Err(AppError::Validation(message)) if message == "Emby Connect password is required")
    );

    app.create_media_server_connection(
        &system_settings_user(),
        draft(EmbyConnectionMode::Local, Some("   "), None),
    )
    .await
    .expect("create local Emby connection");
    app.create_media_server_connection(
        &system_settings_user(),
        draft(EmbyConnectionMode::Connect, None, Some("\t ")),
    )
    .await
    .expect("create Connect Emby connection");

    assert_eq!(&*local_passwords.lock().await, &["   "]);
    assert_eq!(&*connect_passwords.lock().await, &["\t "]);
}

#[tokio::test]
async fn emby_base_url_only_update_persists_verified_canonical_api_root() {
    let app = app_with_repository_and_verifier(
        Arc::new(TestMediaServerConnectionRepository::new(vec![
            emby_connection(true, Some("stored-key")),
        ])),
        Arc::new(EmbySetupVerifier {
            finish_compensation: Arc::new(Mutex::new(Vec::new())),
            local_admin_passwords: Arc::new(Mutex::new(Vec::new())),
            connect_passwords: Arc::new(Mutex::new(Vec::new())),
        }),
    );
    let mut patch = empty_update_patch("emby-main");
    patch.base_url = Some("https://proxy.example.test".into());

    let updated = app
        .update_media_server_connection(&permission_manager_user(), patch)
        .await
        .expect("update Emby base URL");

    assert_eq!(updated.base_url, "https://emby.example.test");
    assert_eq!(updated.api_key.as_deref(), Some("stored-key"));
    assert_eq!(updated.emby_server_id.as_deref(), Some("emby-server-id"));
}

#[tokio::test]
async fn emby_credential_rotation_with_grants_requires_manage_permissions_before_verifier() {
    let local_admin_passwords = Arc::new(Mutex::new(Vec::new()));
    let connect_passwords = Arc::new(Mutex::new(Vec::new()));
    let verifier = Arc::new(EmbySetupVerifier {
        finish_compensation: Arc::new(Mutex::new(Vec::new())),
        local_admin_passwords: Arc::clone(&local_admin_passwords),
        connect_passwords: Arc::clone(&connect_passwords),
    });
    let app = app_with_repository_and_verifier(
        Arc::new(TestMediaServerConnectionRepository::new(vec![
            emby_connection(true, Some("old-key")),
        ])),
        verifier,
    );
    let mut patch = empty_update_patch("emby-main");
    patch.emby_connection_mode = Some(EmbyConnectionMode::Local);
    patch.emby_local_setup_method = Some(EmbyLocalSetupMethod::AdminCredentials);
    patch.admin_username = Some("attacker-admin".into());
    patch.admin_password = Some("attacker-password".into());

    let error = app
        .update_media_server_connection(&system_settings_user(), patch)
        .await
        .expect_err("credential rotation should require ManagePermissions");

    assert_unauthorized(error);
    assert!(local_admin_passwords.lock().await.is_empty());
    assert!(connect_passwords.lock().await.is_empty());
}

#[tokio::test]
async fn emby_credential_rotation_with_grants_allows_permission_manager() {
    let local_admin_passwords = Arc::new(Mutex::new(Vec::new()));
    let connect_passwords = Arc::new(Mutex::new(Vec::new()));
    let app = app_with_repository_and_verifier(
        Arc::new(TestMediaServerConnectionRepository::new(vec![
            emby_connection(true, Some("old-key")),
        ])),
        Arc::new(EmbySetupVerifier {
            finish_compensation: Arc::new(Mutex::new(Vec::new())),
            local_admin_passwords: Arc::clone(&local_admin_passwords),
            connect_passwords: Arc::clone(&connect_passwords),
        }),
    );

    let mut local_patch = empty_update_patch("emby-main");
    local_patch.emby_connection_mode = Some(EmbyConnectionMode::Local);
    local_patch.emby_local_setup_method = Some(EmbyLocalSetupMethod::ApiKey);
    local_patch.api_key = Some("replacement-api-key".into());
    let locally_rotated = app
        .update_media_server_connection(&permission_manager_user(), local_patch)
        .await
        .expect("permission manager should rotate a local Emby API key");
    assert_eq!(
        locally_rotated.api_key.as_deref(),
        Some("replacement-api-key")
    );

    let mut connect_patch = empty_update_patch("emby-main");
    connect_patch.emby_connection_mode = Some(EmbyConnectionMode::Connect);
    connect_patch.emby_connect_enabled = Some(true);
    connect_patch.emby_connect_username_or_email = Some("connect@example.test".into());
    connect_patch.emby_connect_password = Some("connect-password".into());
    connect_patch.emby_connect_server_id = Some("emby-server-id".into());
    let connect_rotated = app
        .update_media_server_connection(&permission_manager_user(), connect_patch)
        .await
        .expect("permission manager should rotate Emby Connect credentials and server");

    assert_eq!(connect_rotated.api_key.as_deref(), Some("connect-key"));
    assert!(connect_rotated.emby_connect_enabled);
    assert_eq!(&*local_admin_passwords.lock().await, &[] as &[String]);
    assert_eq!(&*connect_passwords.lock().await, &["connect-password"]);
}

#[tokio::test]
async fn newly_created_emby_key_is_compensated_when_create_persistence_fails() {
    let finish = Arc::new(Mutex::new(Vec::new()));
    let app = app_with_repository_and_verifier(
        Arc::new(TestMediaServerConnectionRepository::failing(
            Vec::new(),
            true,
            false,
        )),
        Arc::new(EmbySetupVerifier {
            finish_compensation: Arc::clone(&finish),
            local_admin_passwords: Arc::new(Mutex::new(Vec::new())),
            connect_passwords: Arc::new(Mutex::new(Vec::new())),
        }),
    );

    let error = app
        .create_media_server_connection(
            &system_settings_user(),
            MediaServerConnectionDraft {
                provider: MediaServerProvider::Emby,
                display_name: "Emby".into(),
                base_url: "https://emby.example.test".into(),
                external_url: None,
                enabled: true,
                login_enabled: false,
                linking_enabled: false,
                auto_add_enabled: false,
                default_app_permissions: AppPermissionMask::NONE,
                default_library_grants: Vec::new(),
                machine_id: None,
                plex_auth_token: None,
                plex_server_id: None,
                api_key: None,
                admin_username: Some("admin".into()),
                admin_password: Some("password".into()),
                emby_connection_mode: Some(EmbyConnectionMode::Local),
                emby_local_setup_method: Some(EmbyLocalSetupMethod::AdminCredentials),
                emby_connect_enabled: Some(false),
                emby_connect_username_or_email: None,
                emby_connect_password: None,
                emby_connect_server_id: None,
                path_mappings: Vec::new(),
            },
        )
        .await
        .expect_err("repository failure");

    assert!(error.to_string().contains("injected create failure"));
    assert_eq!(&*finish.lock().await, &[true]);
}

#[tokio::test]
async fn newly_created_emby_key_is_compensated_when_rotation_persistence_fails() {
    let finish = Arc::new(Mutex::new(Vec::new()));
    let app = app_with_repository_and_verifier(
        Arc::new(TestMediaServerConnectionRepository::failing(
            vec![emby_connection(true, Some("old-key"))],
            false,
            true,
        )),
        Arc::new(EmbySetupVerifier {
            finish_compensation: Arc::clone(&finish),
            local_admin_passwords: Arc::new(Mutex::new(Vec::new())),
            connect_passwords: Arc::new(Mutex::new(Vec::new())),
        }),
    );
    let mut patch = empty_update_patch("emby-main");
    patch.emby_connection_mode = Some(EmbyConnectionMode::Local);
    patch.emby_local_setup_method = Some(EmbyLocalSetupMethod::AdminCredentials);
    patch.admin_username = Some("admin".into());
    patch.admin_password = Some("password".into());

    let error = app
        .update_media_server_connection(&permission_manager_user(), patch)
        .await
        .expect_err("repository failure");

    assert!(error.to_string().contains("injected update failure"));
    assert_eq!(&*finish.lock().await, &[true]);
}

#[tokio::test]
async fn media_server_update_rejects_enabling_grant_bearing_connection_without_manage_permissions()
{
    let mut connection = grant_bearing_jellyfin_connection();
    connection.enabled = false;
    let app = app_with_connection(connection);
    let mut patch = empty_update_patch("jellyfin-main");
    patch.enabled = Some(true);

    let error = app
        .update_media_server_connection(&system_settings_user(), patch)
        .await
        .expect_err("system settings user should not activate preserved grants");

    assert_unauthorized(error);
}

#[tokio::test]
async fn media_server_update_rejects_enabling_auth_flags_with_grants_without_manage_permissions() {
    let user = system_settings_user();
    for (name, patch) in [
        ("login", {
            let mut connection = grant_bearing_jellyfin_connection();
            connection.login_enabled = false;
            connection.linking_enabled = false;
            connection.auto_add_enabled = false;
            let mut patch = empty_update_patch("jellyfin-main");
            patch.login_enabled = Some(true);
            (connection, patch)
        }),
        ("linking", {
            let mut connection = grant_bearing_jellyfin_connection();
            connection.login_enabled = false;
            connection.linking_enabled = false;
            connection.auto_add_enabled = false;
            let mut patch = empty_update_patch("jellyfin-main");
            patch.linking_enabled = Some(true);
            (connection, patch)
        }),
        ("auto add", {
            let mut connection = grant_bearing_jellyfin_connection();
            connection.auto_add_enabled = false;
            let mut patch = empty_update_patch("jellyfin-main");
            patch.auto_add_enabled = Some(true);
            (connection, patch)
        }),
    ] {
        let app = app_with_connection(patch.0);
        let error = app
            .update_media_server_connection(&user, patch.1)
            .await
            .expect_err(&format!("{name} should require ManagePermissions"));
        assert_unauthorized(error);
    }
}

#[tokio::test]
async fn media_server_update_rejects_auth_identity_changes_with_grants_without_manage_permissions()
{
    let user = system_settings_user();
    for (name, connection, patch) in [
        ("base url", grant_bearing_jellyfin_connection(), {
            let mut patch = empty_update_patch("jellyfin-main");
            patch.base_url = Some("https://other-jellyfin.example.test".to_string());
            patch
        }),
        ("api key", grant_bearing_jellyfin_connection(), {
            let mut patch = empty_update_patch("jellyfin-main");
            patch.api_key = Some("api-key-2".to_string());
            patch
        }),
        ("admin credentials", grant_bearing_jellyfin_connection(), {
            let mut patch = empty_update_patch("jellyfin-main");
            patch.admin_username = Some("admin".to_string());
            patch.admin_password = Some("password".to_string());
            patch
        }),
        ("plex machine", grant_bearing_plex_connection(), {
            let mut patch = empty_update_patch("plex-main");
            patch.machine_id = Some("machine-2".to_string());
            patch
        }),
        ("plex api key", grant_bearing_plex_connection(), {
            let mut patch = empty_update_patch("plex-main");
            patch.api_key = Some("api-key-2".to_string());
            patch
        }),
        (
            "plex clear api key",
            {
                let mut connection = grant_bearing_plex_connection();
                connection.api_key = Some("api-key-1".to_string());
                connection
            },
            {
                let mut patch = empty_update_patch("plex-main");
                patch.clear_api_key = true;
                patch
            },
        ),
        ("Emby API key", emby_connection(true, Some("old-key")), {
            let mut patch = empty_update_patch("emby-main");
            patch.api_key = Some("replacement-key".to_string());
            patch
        }),
        (
            "Emby clear API key",
            emby_connection(true, Some("old-key")),
            {
                let mut patch = empty_update_patch("emby-main");
                patch.clear_api_key = true;
                patch
            },
        ),
        (
            "Emby administrator credentials",
            emby_connection(true, Some("old-key")),
            {
                let mut patch = empty_update_patch("emby-main");
                patch.admin_username = Some("admin".to_string());
                patch.admin_password = Some("password".to_string());
                patch
            },
        ),
        (
            "Emby connection mode",
            emby_connection(true, Some("old-key")),
            {
                let mut patch = empty_update_patch("emby-main");
                patch.emby_connection_mode = Some(EmbyConnectionMode::Local);
                patch
            },
        ),
        (
            "Emby Connect credentials",
            emby_connection(true, Some("old-key")),
            {
                let mut patch = empty_update_patch("emby-main");
                patch.emby_connect_username_or_email = Some("connect@example.test".to_string());
                patch.emby_connect_password = Some("password".to_string());
                patch
            },
        ),
        (
            "Emby Connect enablement",
            emby_connection(true, Some("old-key")),
            {
                let mut patch = empty_update_patch("emby-main");
                patch.emby_connect_enabled = Some(true);
                patch
            },
        ),
        (
            "Emby Connect server ID",
            emby_connection(true, Some("old-key")),
            {
                let mut patch = empty_update_patch("emby-main");
                patch.emby_connect_server_id = Some("other-emby-server".to_string());
                patch
            },
        ),
    ] {
        let app = app_with_connection(connection);
        let error = app
            .update_media_server_connection(&user, patch)
            .await
            .expect_err(&format!("{name} should require ManagePermissions"));
        assert_unauthorized(error);
    }
}

#[tokio::test]
async fn media_server_update_rejects_adding_non_empty_default_grants_even_when_auth_disabled() {
    let mut connection = grant_bearing_jellyfin_connection();
    connection.enabled = false;
    connection.login_enabled = false;
    connection.linking_enabled = false;
    connection.auto_add_enabled = false;
    connection.default_app_permissions = AppPermissionMask::NONE;
    connection.default_library_grants.clear();
    let app = app_with_connection(connection);
    let mut patch = empty_update_patch("jellyfin-main");
    patch.default_app_permissions = Some(AppPermissionMask::from_permissions([
        AppPermission::ManageCatalogSettings,
    ]));

    let error = app
        .update_media_server_connection(&system_settings_user(), patch)
        .await
        .expect_err("adding default grants should require ManagePermissions");

    assert_unauthorized(error);
}

#[tokio::test]
async fn media_server_update_allows_permission_manager_to_activate_preserved_grants() {
    let mut connection = grant_bearing_jellyfin_connection();
    connection.enabled = false;
    let app = app_with_connection(connection);
    let mut patch = empty_update_patch("jellyfin-main");
    patch.enabled = Some(true);

    let updated = app
        .update_media_server_connection(&permission_manager_user(), patch)
        .await
        .expect("permission manager should activate preserved grants");

    assert!(updated.enabled);
}

#[tokio::test]
async fn media_server_update_allows_system_settings_user_to_deactivate_or_clear_grants() {
    let app = app_with_connection(grant_bearing_jellyfin_connection());
    let mut patch = empty_update_patch("jellyfin-main");
    patch.enabled = Some(false);
    let updated = app
        .update_media_server_connection(&system_settings_user(), patch)
        .await
        .expect("system settings user should be able to deactivate connection");
    assert!(!updated.enabled);

    let app = app_with_connection(grant_bearing_jellyfin_connection());
    let mut patch = empty_update_patch("jellyfin-main");
    patch.default_app_permissions = Some(AppPermissionMask::NONE);
    patch.default_library_grants = Some(Vec::new());
    let updated = app
        .update_media_server_connection(&system_settings_user(), patch)
        .await
        .expect("system settings user should be able to clear grants");
    assert!(updated.default_app_permissions.is_empty());
    assert!(updated.default_library_grants.is_empty());
}

#[tokio::test]
async fn media_server_update_allows_harmless_save_with_unchanged_grants_without_manage_permissions()
{
    let connection = grant_bearing_jellyfin_connection();
    let app = app_with_connection(connection.clone());
    let mut patch = empty_update_patch("jellyfin-main");
    patch.display_name = Some("Home Jellyfin".to_string());
    patch.base_url = Some(connection.base_url);
    patch.enabled = Some(connection.enabled);
    patch.login_enabled = Some(connection.login_enabled);
    patch.linking_enabled = Some(connection.linking_enabled);
    patch.auto_add_enabled = Some(connection.auto_add_enabled);
    patch.default_app_permissions = Some(connection.default_app_permissions);
    patch.default_library_grants = Some(connection.default_library_grants);

    let updated = app
        .update_media_server_connection(&system_settings_user(), patch)
        .await
        .expect("unchanged grant payload should not require ManagePermissions");

    assert_eq!(updated.display_name, "Home Jellyfin");
    assert!(!updated.default_app_permissions.is_empty());
    assert!(!updated.default_library_grants.is_empty());
}

#[tokio::test]
async fn media_server_create_preserves_plex_token_and_path_mappings() {
    let app = app_with_connections(Vec::new());
    let created = app
        .create_media_server_connection(
            &system_settings_user(),
            MediaServerConnectionDraft {
                provider: MediaServerProvider::Plex,
                display_name: "Plex".to_string(),
                base_url: "http://plex:32400".to_string(),
                external_url: None,
                enabled: true,
                login_enabled: false,
                linking_enabled: false,
                auto_add_enabled: false,
                default_app_permissions: AppPermissionMask::NONE,
                default_library_grants: Vec::new(),
                machine_id: None,
                plex_auth_token: Some(" plex-token ".to_string()),
                plex_server_id: None,
                api_key: None,
                admin_username: None,
                admin_password: None,
                emby_connection_mode: None,
                emby_local_setup_method: None,
                emby_connect_enabled: None,
                emby_connect_username_or_email: None,
                emby_connect_password: None,
                emby_connect_server_id: None,
                path_mappings: vec![MediaServerPathMapping {
                    source_path: "/mnt/plex".to_string(),
                    destination_path: "/data/media".to_string(),
                    sort_order: 0,
                }],
            },
        )
        .await
        .expect("Plex connection should be created");

    assert_eq!(created.api_key.as_deref(), Some("plex-token"));
    assert_eq!(created.base_url, "http://plex:32400");
    assert_eq!(
        created.path_mappings,
        vec![MediaServerPathMapping {
            source_path: "/mnt/plex".to_string(),
            destination_path: "/data/media".to_string(),
            sort_order: 0,
        }]
    );
}

#[tokio::test]
async fn media_server_update_preserves_existing_plex_token_and_replaces_from_oauth() {
    let mut connection = grant_bearing_plex_connection();
    connection.api_key = Some("old-token".to_string());
    let app = app_with_connection(connection);
    let updated = app
        .update_media_server_connection(&permission_manager_user(), {
            let mut patch = empty_update_patch("plex-main");
            patch.plex_auth_token = Some(" new-token ".to_string());
            patch.path_mappings = Some(vec![MediaServerPathMapping {
                source_path: "/mnt/plex".to_string(),
                destination_path: "/data/media".to_string(),
                sort_order: 0,
            }]);
            patch
        })
        .await
        .expect("Plex connection should update");

    assert_eq!(updated.api_key.as_deref(), Some("new-token"));
    assert_eq!(updated.base_url, "https://plex.tv");
    assert_eq!(updated.path_mappings.len(), 1);

    let app = app_with_connection(updated);
    let unchanged = app
        .update_media_server_connection(&permission_manager_user(), empty_update_patch("plex-main"))
        .await
        .expect("empty update should preserve Plex token");

    assert_eq!(unchanged.api_key.as_deref(), Some("new-token"));
    assert_eq!(unchanged.base_url, "https://plex.tv");
    assert_eq!(unchanged.path_mappings.len(), 1);
}

#[tokio::test]
async fn media_server_update_does_not_carry_api_key_across_provider_change() {
    let mut connection = grant_bearing_jellyfin_connection();
    connection.login_enabled = false;
    connection.auto_add_enabled = false;
    connection.default_app_permissions = AppPermissionMask::NONE;
    connection.default_library_grants.clear();
    connection.api_key = Some("jellyfin-token".to_string());
    let app = app_with_connection(connection);

    let updated = app
        .update_media_server_connection(&system_settings_user(), {
            let mut patch = empty_update_patch("jellyfin-main");
            patch.provider = Some(MediaServerProvider::Plex);
            patch.display_name = Some("Plex".to_string());
            patch.base_url = Some("http://plex:32400".to_string());
            patch.login_enabled = Some(false);
            patch.linking_enabled = Some(false);
            patch.auto_add_enabled = Some(false);
            patch.default_app_permissions = Some(AppPermissionMask::NONE);
            patch.default_library_grants = Some(Vec::new());
            patch.path_mappings = Some(Vec::new());
            patch
        })
        .await
        .expect("provider change should not reuse old provider secret");

    assert_eq!(updated.provider, MediaServerProvider::Plex);
    assert_eq!(updated.api_key, None);
}

#[tokio::test]
async fn plex_media_server_notification_channel_uses_facade_config() {
    let mut connection = grant_bearing_plex_connection();
    connection.api_key = Some("plex-token".to_string());
    connection.path_mappings = vec![MediaServerPathMapping {
        source_path: "/mnt/plex".to_string(),
        destination_path: "/data/media".to_string(),
        sort_order: 0,
    }];
    let app = app_with_connection(connection);

    let channel = app
        .notification_channel_for_media_server_target("plex-main")
        .await
        .expect("Plex media server notification channel should resolve");
    let config: serde_json::Value =
        serde_json::from_str(&channel.config_json).expect("config should be JSON");

    assert_eq!(channel.id, "media-server:plex-main");
    assert_eq!(channel.channel_type.as_str(), "plex");
    assert_eq!(config["base_url"], "https://plex.tv");
    assert_eq!(config["api_key"], "plex-token");
    assert_eq!(config["machine_id"], "machine-1");
    assert_eq!(config["path_mappings"], "/data/media => /mnt/plex");
}

#[tokio::test]
async fn playback_mapping_upsert_preserves_untouched_items_and_full_replace_removes_stale_items() {
    let repository = TestMediaServerConnectionRepository::new(Vec::new());
    let now = Utc::now();
    let title = MediaServerPlaybackItem {
        connection_id: "server-1".into(),
        entity_kind: MediaServerPlaybackEntityKind::Title,
        entity_id: "title-1".into(),
        provider_item_id: "provider-title-1".into(),
        last_seen_at: now,
    };
    let stale_episode = MediaServerPlaybackItem {
        connection_id: "server-1".into(),
        entity_kind: MediaServerPlaybackEntityKind::Episode,
        entity_id: "episode-1".into(),
        provider_item_id: "provider-episode-old".into(),
        last_seen_at: now,
    };
    repository
        .replace_playback_items_for_connection(
            "server-1",
            vec![title.clone(), stale_episode.clone()],
        )
        .await
        .expect("seed playback mappings");

    let mut refreshed_episode = stale_episode.clone();
    refreshed_episode.provider_item_id = "provider-episode-new".into();
    repository
        .upsert_playback_items_for_connection("server-1", vec![refreshed_episode.clone()])
        .await
        .expect("incremental upsert");
    let incrementally_refreshed_title = repository
        .list_playback_items_for_entity(MediaServerPlaybackEntityKind::Title, "title-1")
        .await
        .expect("list incrementally refreshed title mappings");
    let incrementally_refreshed_episode = repository
        .list_playback_items_for_entity(MediaServerPlaybackEntityKind::Episode, "episode-1")
        .await
        .expect("list incrementally refreshed episode mappings");
    assert!(incrementally_refreshed_title.contains(&title));
    assert!(incrementally_refreshed_episode.contains(&refreshed_episode));

    repository
        .replace_playback_items_for_connection("server-1", vec![title.clone()])
        .await
        .expect("full reconciliation");
    assert_eq!(
        repository
            .list_playback_items_for_entity(MediaServerPlaybackEntityKind::Title, "title-1")
            .await
            .expect("list reconciled mappings"),
        vec![title]
    );
    assert!(
        repository
            .list_playback_items_for_entity(MediaServerPlaybackEntityKind::Episode, "episode-1")
            .await
            .expect("list stale episode mappings")
            .is_empty()
    );
}
