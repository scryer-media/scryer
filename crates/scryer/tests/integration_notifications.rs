#![recursion_limit = "256"]

mod common;

use async_graphql::Request;
use async_trait::async_trait;
use chrono::Utc;
use common::{TestContext, disabled_auth_runtime_handle, initialize_wasm_runtime_for_tests};
use scryer_application::testing::AppUseCaseTestExt;
use scryer_application::{
    AppError, AppResult, MediaServerConnectionRepository, NotificationAppPayload,
    NotificationClient, NotificationExternalIdsPayload, NotificationFilePayload,
    NotificationMediaFilePayload, NotificationMediaUpdatePayload,
    NotificationMediaUpdateTypePayload, NotificationPayload, NotificationPluginProvider,
    NotificationScopeIdUpdate, NotificationSubscriptionTargetCreate, NotificationTitlePayload,
    start_notification_dispatcher,
};
use scryer_domain::{
    AppPermissionMask, ConfigFieldDef, ConfigFieldOption, ConfigFieldType, ConfigFieldValueSource,
    DomainEventActorKind, DomainEventPayload, DomainEventStream, DomainEventType,
    DomainExternalIds, ExternalId, ImportCompletedEventData, LibraryScanProgressedEventData,
    MediaFacet, MediaFileDeletedEventData, MediaFileDeletedReason, MediaFileRenamedEventData,
    MediaFileUpgradedEventData, MediaPathUpdate, MediaRequestSubmittedEventData,
    MediaServerConnection, MediaServerPathMapping, MediaServerProvider, MediaUpdateType,
    NewDomainEvent, NewTitle, NotificationChannelConfig, NotificationEventType,
    TitleContextSnapshot,
};
use scryer_infrastructure_library::media::servers::MediaServerConnectionStore;
use scryer_infrastructure_notifications::notifications::store::NotificationStore;
use scryer_interface::build_schema;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast::error::TryRecvError;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Wire notification repos into the test AppUseCase so CRUD methods don't
/// return "not configured".
fn app_with_notifications(ctx: &TestContext) -> scryer_application::AppUseCase {
    ctx.app.with_test_overrides(|builder| {
        builder
            .with_notification_store(Arc::new(NotificationStore::new(
                ctx.db.datastore(),
                ctx.db.encryption_key_state(),
            )))
            .with_media_server_connection_store(Arc::new(MediaServerConnectionStore::new(
                ctx.db.datastore(),
                ctx.db.encryption_key_state(),
            )))
    })
}

fn app_with_notification_provider(
    ctx: &TestContext,
    provider: Arc<dyn NotificationPluginProvider>,
) -> scryer_application::AppUseCase {
    app_with_notifications(ctx)
        .with_test_overrides(|builder| builder.with_notification_provider(provider))
}

async fn default_user(app: &scryer_application::AppUseCase) -> scryer_domain::User {
    app.find_or_create_default_user().await.unwrap()
}

fn media_server_connection_store(ctx: &TestContext) -> MediaServerConnectionStore {
    MediaServerConnectionStore::new(ctx.db.datastore(), ctx.db.encryption_key_state())
}

async fn insert_jellyfin_media_server_connection(
    ctx: &TestContext,
    id: &str,
    base_url: &str,
    path_mappings: Vec<MediaServerPathMapping>,
) -> MediaServerConnection {
    insert_jellyfin_media_server_connection_with_api_key(
        ctx,
        id,
        base_url,
        Some("secret".to_string()),
        path_mappings,
    )
    .await
}

async fn insert_jellyfin_media_server_connection_with_api_key(
    ctx: &TestContext,
    id: &str,
    base_url: &str,
    api_key: Option<String>,
    path_mappings: Vec<MediaServerPathMapping>,
) -> MediaServerConnection {
    let now = Utc::now();
    let connection = MediaServerConnection {
        id: id.to_string(),
        provider: MediaServerProvider::Jellyfin,
        display_name: "Jellyfin".to_string(),
        base_url: base_url.trim_end_matches('/').to_string(),
        external_url: None,
        enabled: true,
        login_enabled: false,
        linking_enabled: false,
        auto_add_enabled: false,
        default_app_permissions: AppPermissionMask::from_bits_retain(0),
        default_library_grants: Vec::new(),
        machine_id: None,
        api_key,
        emby_server_id: None,
        emby_connect_enabled: false,
        path_mappings,
        created_at: now,
        updated_at: now,
    };
    media_server_connection_store(ctx)
        .create(connection)
        .await
        .expect("media server connection should insert")
}

fn jellyfin_path_mappings() -> Vec<MediaServerPathMapping> {
    vec![
        MediaServerPathMapping {
            source_path: "/data/Movies".to_string(),
            destination_path: "/mnt/media/Movies".to_string(),
            sort_order: 0,
        },
        MediaServerPathMapping {
            source_path: "/data/TV".to_string(),
            destination_path: "/mnt/media/TV".to_string(),
            sort_order: 1,
        },
    ]
}

async fn create_media_server_subscription(
    app: &scryer_application::AppUseCase,
    user: &scryer_domain::User,
    connection: &MediaServerConnection,
    event_type: &str,
) {
    app.create_notification_subscription_for_target(
        user,
        NotificationSubscriptionTargetCreate {
            channel_id: None,
            target_kind: Some("media_server_connection".into()),
            target_id: Some(connection.id.clone()),
            event_type: event_type.to_string(),
            scope: "global".into(),
            scope_id: None,
            is_enabled: true,
        },
    )
    .await
    .expect("create media server target subscription");
}

#[derive(Debug, Clone, PartialEq)]
struct CapturedNotification {
    event_type: String,
    title: String,
    message: String,
    metadata: HashMap<String, Value>,
}

#[derive(Clone)]
struct FakeNotificationClient {
    captured: Arc<Mutex<Vec<CapturedNotification>>>,
}

#[async_trait]
impl NotificationClient for FakeNotificationClient {
    async fn send_notification(&self, payload: &NotificationPayload) -> AppResult<()> {
        self.captured.lock().unwrap().push(CapturedNotification {
            event_type: payload.event_type.as_str().to_string(),
            title: payload.summary_title.clone(),
            message: payload.summary_message.clone(),
            metadata: captured_metadata(payload),
        });
        Ok(())
    }
}

#[derive(Clone)]
struct FakeNotificationProvider {
    provider_type: String,
    provider_name: String,
    config_fields: Vec<ConfigFieldDef>,
    supports_test: bool,
    captured: Arc<Mutex<Vec<CapturedNotification>>>,
}

impl FakeNotificationProvider {
    fn webhook() -> Self {
        Self {
            provider_type: "webhook".to_string(),
            provider_name: "Webhook".to_string(),
            config_fields: Vec::new(),
            supports_test: true,
            captured: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn webhook_without_test() -> Self {
        Self {
            supports_test: false,
            ..Self::webhook()
        }
    }

    fn jellyfin() -> Self {
        Self {
            provider_type: "jellyfin".to_string(),
            provider_name: "Jellyfin".to_string(),
            config_fields: vec![
                ConfigFieldDef {
                    key: "base_url".to_string(),
                    label: "Base URL".to_string(),
                    field_type: ConfigFieldType::String,
                    required: true,
                    default_value: None,
                    value_source: ConfigFieldValueSource::User,
                    role: None,
                    host_binding: None,
                    options: vec![],
                    help_text: None,
                    ..Default::default()
                },
                ConfigFieldDef {
                    key: "api_key".to_string(),
                    label: "API Key".to_string(),
                    field_type: ConfigFieldType::Password,
                    required: true,
                    default_value: None,
                    value_source: ConfigFieldValueSource::User,
                    role: None,
                    host_binding: None,
                    options: vec![],
                    help_text: None,
                    ..Default::default()
                },
                ConfigFieldDef {
                    key: "path_mappings".to_string(),
                    label: "Path Mappings".to_string(),
                    field_type: ConfigFieldType::Multiline,
                    required: false,
                    default_value: None,
                    value_source: ConfigFieldValueSource::User,
                    role: None,
                    host_binding: None,
                    options: vec![ConfigFieldOption {
                        value: "/data => /mnt".to_string(),
                        label: "Example".to_string(),
                        config_overrides: Default::default(),
                    }],
                    help_text: Some("One mapping per line.".to_string()),
                    ..Default::default()
                },
            ],
            supports_test: true,
            captured: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn captured(&self) -> Vec<CapturedNotification> {
        self.captured.lock().unwrap().clone()
    }
}

fn jellyfin_supported_event_types() -> Vec<NotificationEventType> {
    vec![
        NotificationEventType::ImportComplete,
        NotificationEventType::Upgrade,
        NotificationEventType::Rename,
        NotificationEventType::FileDeleted,
        NotificationEventType::FileDeletedForUpgrade,
    ]
}

impl NotificationPluginProvider for FakeNotificationProvider {
    fn client_for_channel(
        &self,
        config: &scryer_domain::NotificationChannelConfig,
    ) -> Option<Arc<dyn NotificationClient>> {
        if config.channel_type.as_str() != self.provider_type {
            return None;
        }

        Some(Arc::new(FakeNotificationClient {
            captured: Arc::clone(&self.captured),
        }))
    }

    fn available_provider_types(&self) -> Vec<String> {
        vec![self.provider_type.clone()]
    }

    fn config_fields_for_provider(&self, provider_type: &str) -> Vec<ConfigFieldDef> {
        if provider_type == self.provider_type {
            self.config_fields.clone()
        } else {
            vec![]
        }
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        (provider_type == self.provider_type).then(|| self.provider_name.clone())
    }

    fn supported_events_for_provider(&self, provider_type: &str) -> Vec<NotificationEventType> {
        if provider_type == self.provider_type && self.provider_type == "jellyfin" {
            jellyfin_supported_event_types()
        } else {
            vec![]
        }
    }

    fn supports_test_for_provider(&self, provider_type: &str) -> bool {
        provider_type == self.provider_type && self.supports_test
    }
}

fn assert_no_errors(body: &Value) {
    assert!(
        body.get("errors").is_none(),
        "unexpected GraphQL errors: {body}"
    );
}

async fn schema_exec(
    app: &scryer_application::AppUseCase,
    _ctx: &TestContext,
    query: &str,
) -> Value {
    let schema = build_schema(app.clone(), disabled_auth_runtime_handle());
    let user = default_user(app).await;
    let response = schema.execute(Request::new(query).data(user)).await;
    serde_json::to_value(&response).expect("serialize GraphQL response")
}

fn jellyfin_config_json(base_url: &str, path_mappings: &str) -> String {
    serde_json::json!({
        "base_url": base_url,
        "api_key": "secret",
        "path_mappings": path_mappings,
    })
    .to_string()
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("repo root")
        .to_path_buf()
}

fn jellyfin_dist_wasm_path() -> PathBuf {
    repo_root()
        .parent()
        .expect("workspace root")
        .join("scryer-plugins")
        .join("dist")
        .join("jellyfin_notification.wasm")
}

fn load_jellyfin_dist_provider() -> Option<Arc<dyn NotificationPluginProvider>> {
    initialize_wasm_runtime_for_tests();
    let wasm_path = jellyfin_dist_wasm_path();
    if !wasm_path.exists() {
        eprintln!(
            "skipping jellyfin dist test; missing {}",
            wasm_path.display()
        );
        return None;
    }

    let wasm_bytes = std::fs::read(&wasm_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", wasm_path.display()));
    let wasm_provider =
        scryer_plugins::WasmNotificationPluginProvider::empty().with_external_bytes(&wasm_bytes);
    if !wasm_provider
        .available_provider_types()
        .iter()
        .any(|provider_type| provider_type == "jellyfin")
    {
        eprintln!(
            "skipping jellyfin dist test; optional dist artifact is incompatible with SDK {}",
            scryer_plugins::SDK_VERSION
        );
        return None;
    }

    Some(Arc::new(
        scryer_plugins::DynamicNotificationPluginProvider::new(wasm_provider),
    ))
}

fn jellyfin_channel_config(base_url: &str, path_mappings: &str) -> NotificationChannelConfig {
    let now = Utc::now();
    NotificationChannelConfig {
        id: "channel-jellyfin".to_string(),
        name: "Jellyfin".to_string(),
        channel_type: scryer_domain::ChannelType::parse("jellyfin").expect("jellyfin channel"),
        config_json: jellyfin_config_json(base_url, path_mappings),
        media_server_connection_id: None,
        is_enabled: true,
        created_at: now,
        updated_at: now,
    }
}

fn jellyfin_title_payload(
    name: &str,
    facet: &str,
    path: Option<&str>,
    external_ids: NotificationExternalIdsPayload,
) -> NotificationTitlePayload {
    NotificationTitlePayload {
        id: Some("title-1".to_string()),
        name: name.to_string(),
        facet: facet.to_string(),
        year: Some(2024),
        slug: Some("example-title".to_string()),
        path: path.map(str::to_string),
        overview: None,
        sort_title: None,
        poster_url: None,
        background_url: None,
        tags: Vec::new(),
        aliases: Vec::new(),
        original_language: None,
        original_country: None,
        external_ids,
    }
}

fn jellyfin_notification_payload(
    event_type: NotificationEventType,
    title: Option<NotificationTitlePayload>,
    file: Option<NotificationFilePayload>,
    media_files: Vec<NotificationMediaFilePayload>,
) -> NotificationPayload {
    NotificationPayload {
        schema_version: 1,
        event_type,
        event_id: Some("evt-jellyfin".to_string()),
        occurred_at: Some(Utc::now().to_rfc3339()),
        correlation_id: None,
        actor: None,
        severity: None,
        is_test: false,
        summary_title: "Jellyfin Test".to_string(),
        summary_message: "Jellyfin notification test payload".to_string(),
        app: NotificationAppPayload {
            name: "Scryer".to_string(),
            version: "test".to_string(),
        },
        title,
        episode: None,
        episodes: Vec::new(),
        release: None,
        download: None,
        import: None,
        health: None,
        file,
        media_files,
        application_update: None,
        manual_interaction: None,
        media_request: None,
    }
}

fn test_notification_payload() -> NotificationPayload {
    NotificationPayload {
        schema_version: 1,
        event_type: NotificationEventType::Test,
        event_id: None,
        occurred_at: None,
        correlation_id: None,
        actor: None,
        severity: None,
        is_test: true,
        summary_title: "Scryer Test Notification".to_string(),
        summary_message: "This is a test notification from Scryer.".to_string(),
        app: NotificationAppPayload {
            name: "Scryer".to_string(),
            version: "test".to_string(),
        },
        title: None,
        episode: None,
        episodes: Vec::new(),
        release: None,
        download: None,
        import: None,
        health: None,
        file: None,
        media_files: Vec::new(),
        application_update: None,
        manual_interaction: None,
        media_request: None,
    }
}

fn lifecycle_metadata(
    title_name: &str,
    facet: &str,
    updates: Vec<(&str, &str)>,
    external_ids: Value,
) -> HashMap<String, Value> {
    let media_updates = updates
        .iter()
        .map(|(path, update_type)| {
            json!({
                "path": path,
                "update_type": update_type,
            })
        })
        .collect::<Vec<_>>();

    HashMap::from([
        ("title_name".to_string(), json!(title_name)),
        ("title_facet".to_string(), json!(facet)),
        ("file_path".to_string(), json!(updates[0].0)),
        ("media_updates".to_string(), Value::Array(media_updates)),
        ("external_ids".to_string(), external_ids),
    ])
}

fn captured_metadata(payload: &NotificationPayload) -> HashMap<String, Value> {
    let mut metadata = HashMap::new();

    if let Some(title) = &payload.title {
        metadata.insert("title_name".to_string(), json!(title.name));
        metadata.insert("title_facet".to_string(), json!(title.facet));
        if let Some(year) = title.year {
            metadata.insert("title_year".to_string(), json!(year));
        }
        if !title.tags.is_empty() {
            metadata.insert("title_tags".to_string(), json!(title.tags));
        }

        let mut external_ids = serde_json::Map::new();
        if let Some(tmdb_id) = &title.external_ids.tmdb_id {
            external_ids.insert("tmdb_id".to_string(), json!(tmdb_id));
        }
        if let Some(imdb_id) = &title.external_ids.imdb_id {
            external_ids.insert("imdb_id".to_string(), json!(imdb_id));
        }
        if let Some(tvdb_id) = &title.external_ids.tvdb_id {
            external_ids.insert("tvdb_id".to_string(), json!(tvdb_id));
        }
        if let Some(anidb_id) = &title.external_ids.anidb_id {
            external_ids.insert("anidb_id".to_string(), json!(anidb_id));
        }
        metadata.insert("external_ids".to_string(), Value::Object(external_ids));

        if !title.external_ids.by_source.is_empty()
            && title
                .external_ids
                .by_source
                .keys()
                .any(|source| !matches!(source.as_str(), "tmdb" | "imdb" | "tvdb" | "anidb"))
        {
            metadata.insert(
                "external_ids_by_source".to_string(),
                json!(title.external_ids.by_source),
            );
        }
    }

    if let Some(episode) = payload.episodes.first() {
        metadata.insert("episode_id".to_string(), json!(episode.id));
        if let Some(season_number) = &episode.season_number {
            metadata.insert("episode_season_number".to_string(), json!(season_number));
        }
        if let Some(episode_number) = &episode.episode_number {
            metadata.insert("episode_number".to_string(), json!(episode_number));
        }
        if let Some(title) = &episode.title {
            metadata.insert("episode_title".to_string(), json!(title));
        }
        if let Some(air_date) = &episode.air_date {
            metadata.insert("episode_air_date".to_string(), json!(air_date));
        }
    }

    if let Some(file) = &payload.file {
        if let Some(primary_path) = &file.primary_path {
            metadata.insert("file_path".to_string(), json!(primary_path));
        }

        let media_updates = file
            .media_updates
            .iter()
            .map(|update| {
                json!({
                    "path": update.path,
                    "update_type": match update.update_type {
                        NotificationMediaUpdateTypePayload::Created => "created",
                        NotificationMediaUpdateTypePayload::Modified => "modified",
                        NotificationMediaUpdateTypePayload::Deleted => "deleted",
                    },
                })
            })
            .collect::<Vec<_>>();
        metadata.insert("media_updates".to_string(), Value::Array(media_updates));
    }

    metadata
}

fn import_completed_event_data(
    title: TitleContextSnapshot,
    media_updates: Vec<MediaPathUpdate>,
    imported_count: i32,
    episode_ids: Vec<String>,
) -> ImportCompletedEventData {
    ImportCompletedEventData {
        title,
        media_updates,
        imported_count,
        import_id: None,
        source_system: None,
        source_ref: None,
        source_title: None,
        source_path: None,
        dest_path: None,
        quality: None,
        episode_ids,
        size_bytes: None,
    }
}

fn title_context(
    title_name: &str,
    facet: &str,
    external_ids: DomainExternalIds,
) -> TitleContextSnapshot {
    TitleContextSnapshot {
        title_name: title_name.to_string(),
        facet: MediaFacet::parse(facet).expect("valid facet"),
        external_ids,
        poster_url: None,
        year: None,
    }
}

fn external_id(source: &str, value: &str) -> ExternalId {
    ExternalId {
        source: source.to_string(),
        value: value.to_string(),
    }
}

fn new_event(
    event_id: &str,
    title_id: &str,
    facet: &str,
    payload: DomainEventPayload,
) -> NewDomainEvent {
    NewDomainEvent {
        event_id: event_id.to_string(),
        occurred_at: Utc::now(),
        actor_kind: DomainEventActorKind::User,
        actor_user_id: Some("user-1".to_string()),
        actor_display_name: "user-1".to_string(),
        title_id: Some(title_id.to_string()),
        facet: MediaFacet::parse(facet),
        correlation_id: None,
        causation_id: None,
        schema_version: 1,
        stream: DomainEventStream::Title {
            title_id: title_id.to_string(),
        },
        payload,
    }
}

async fn wait_for_captured(
    provider: &FakeNotificationProvider,
    expected: usize,
) -> Vec<CapturedNotification> {
    for _ in 0..50 {
        let captured = provider.captured();
        if captured.len() >= expected {
            return captured;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    panic!(
        "timed out waiting for {expected} notifications, captured {:?}",
        provider.captured()
    );
}

async fn wait_for_wiremock_requests(
    server: &wiremock::MockServer,
    expected: usize,
) -> Vec<wiremock::Request> {
    for _ in 0..50 {
        let requests = server
            .received_requests()
            .await
            .expect("request capture should succeed");
        if requests.len() >= expected {
            return requests;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let requests = server
        .received_requests()
        .await
        .expect("request capture should succeed");
    panic!("timed out waiting for {expected} HTTP requests, captured {requests:?}");
}

// ---------------------------------------------------------------------------
// Channel CRUD
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_and_list_channels() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let ch = app
        .create_notification_channel(&user, "Discord".into(), "webhook".into(), "{}".into(), true)
        .await
        .expect("create channel");
    assert_eq!(ch.name, "Discord");
    assert_eq!(ch.channel_type.as_str(), "webhook");
    assert!(ch.is_enabled);

    let channels = app.list_notification_channels(&user).await.expect("list");
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0].id, ch.id);
}

#[tokio::test]
async fn get_channel_by_id() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let ch = app
        .create_notification_channel(&user, "Slack".into(), "webhook".into(), "{}".into(), false)
        .await
        .unwrap();

    let fetched = app
        .get_notification_channel(&user, &ch.id)
        .await
        .unwrap()
        .expect("should find channel");
    assert_eq!(fetched.name, "Slack");
    assert!(!fetched.is_enabled);
}

#[tokio::test]
async fn update_channel() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let ch = app
        .create_notification_channel(
            &user,
            "Old Name".into(),
            "webhook".into(),
            "{\"url\":\"http://a\"}".into(),
            true,
        )
        .await
        .unwrap();

    let updated = app
        .update_notification_channel(
            &user,
            ch.id.clone(),
            Some("New Name".into()),
            Some("{\"url\":\"http://b\"}".into()),
            Some(false),
        )
        .await
        .unwrap();

    assert_eq!(updated.name, "New Name");
    assert_eq!(updated.config_json, "{\"url\":\"http://b\"}");
    assert!(!updated.is_enabled);
}

#[tokio::test]
async fn delete_channel() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let ch = app
        .create_notification_channel(&user, "Temp".into(), "webhook".into(), "{}".into(), true)
        .await
        .unwrap();

    app.delete_notification_channel(&user, &ch.id)
        .await
        .expect("delete");

    let channels = app.list_notification_channels(&user).await.unwrap();
    assert!(channels.is_empty());
}

// ---------------------------------------------------------------------------
// Channel validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_channel_rejects_empty_name() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let err = app
        .create_notification_channel(&user, "".into(), "webhook".into(), "{}".into(), true)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
}

#[tokio::test]
async fn create_channel_rejects_empty_type() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let err = app
        .create_notification_channel(&user, "Slack".into(), "  ".into(), "{}".into(), true)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
}

#[tokio::test]
async fn create_channel_rejects_non_object_config_json() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let err = app
        .create_notification_channel(&user, "Webhook".into(), "webhook".into(), "[]".into(), true)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
}

#[tokio::test]
async fn test_channel_rejects_provider_without_test_capability() {
    let ctx = TestContext::new().await;
    let provider = Arc::new(FakeNotificationProvider::webhook_without_test());
    let app = app_with_notification_provider(&ctx, provider.clone());
    let user = default_user(&app).await;
    let channel = app
        .create_notification_channel(&user, "Webhook".into(), "webhook".into(), "{}".into(), true)
        .await
        .expect("channel should be created");

    let err = app
        .test_notification_channel(&user, &channel.id)
        .await
        .unwrap_err();

    assert!(matches!(err, AppError::Validation(_)));
    assert!(provider.captured().is_empty());
}

#[tokio::test]
async fn create_channel_rejects_media_server_provider_type() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let err = app
        .create_notification_channel(
            &user,
            "Jellyfin".into(),
            "  Jellyfin  ".into(),
            "{}".into(),
            true,
        )
        .await
        .expect_err("media server channels must be managed in Media Servers");

    assert!(matches!(err, AppError::Validation(_)));
}

#[tokio::test]
async fn update_nonexistent_channel_returns_not_found() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let err = app
        .update_notification_channel(&user, "nonexistent".into(), Some("x".into()), None, None)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
}

// ---------------------------------------------------------------------------
// Subscription CRUD
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_and_list_subscriptions() {
    let ctx = TestContext::new().await;
    let app = app_with_notification_provider(&ctx, Arc::new(FakeNotificationProvider::webhook()));
    let user = default_user(&app).await;

    let ch = app
        .create_notification_channel(&user, "Discord".into(), "webhook".into(), "{}".into(), true)
        .await
        .unwrap();

    let sub = app
        .create_notification_subscription(
            &user,
            ch.id.clone(),
            "release_grabbed".into(),
            "global".into(),
            None,
            true,
        )
        .await
        .expect("create subscription");

    assert_eq!(sub.channel_id.as_deref(), Some(ch.id.as_str()));
    assert_eq!(sub.event_type, NotificationEventType::Grab);
    assert!(sub.is_enabled);

    let subs = app.list_notification_subscriptions(&user).await.unwrap();
    assert_eq!(subs.len(), 1);
}

#[tokio::test]
async fn update_subscription() {
    let ctx = TestContext::new().await;
    let app = app_with_notification_provider(&ctx, Arc::new(FakeNotificationProvider::webhook()));
    let user = default_user(&app).await;

    let ch = app
        .create_notification_channel(&user, "Ch".into(), "webhook".into(), "{}".into(), true)
        .await
        .unwrap();

    let sub = app
        .create_notification_subscription(
            &user,
            ch.id.clone(),
            "release_grabbed".into(),
            "global".into(),
            None,
            true,
        )
        .await
        .unwrap();

    let updated = app
        .update_notification_subscription(
            &user,
            sub.id.clone(),
            Some("import_completed".into()),
            None,
            NotificationScopeIdUpdate::NoChange,
            Some(false),
        )
        .await
        .unwrap();

    assert_eq!(updated.event_type, NotificationEventType::ImportComplete);
    assert!(!updated.is_enabled);
}

#[tokio::test]
async fn delete_subscription() {
    let ctx = TestContext::new().await;
    let app = app_with_notification_provider(&ctx, Arc::new(FakeNotificationProvider::webhook()));
    let user = default_user(&app).await;

    let ch = app
        .create_notification_channel(&user, "Ch".into(), "webhook".into(), "{}".into(), true)
        .await
        .unwrap();

    let sub = app
        .create_notification_subscription(
            &user,
            ch.id,
            "release_grabbed".into(),
            "global".into(),
            None,
            true,
        )
        .await
        .unwrap();

    app.delete_notification_subscription(&user, &sub.id)
        .await
        .expect("delete");

    let subs = app.list_notification_subscriptions(&user).await.unwrap();
    assert!(subs.is_empty());
}

// ---------------------------------------------------------------------------
// Subscription validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_subscription_rejects_unknown_event_type() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let ch = app
        .create_notification_channel(&user, "Ch".into(), "webhook".into(), "{}".into(), true)
        .await
        .unwrap();

    let err = app
        .create_notification_subscription(
            &user,
            ch.id,
            "nonexistent_event".into(),
            "global".into(),
            None,
            true,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
}

#[tokio::test]
async fn create_subscription_rejects_nonexistent_channel() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let err = app
        .create_notification_subscription(
            &user,
            "nonexistent-channel".into(),
            "release_grabbed".into(),
            "global".into(),
            None,
            true,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
}

#[tokio::test]
async fn update_subscription_rejects_unknown_event_type() {
    let ctx = TestContext::new().await;
    let app = app_with_notification_provider(&ctx, Arc::new(FakeNotificationProvider::webhook()));
    let user = default_user(&app).await;

    let ch = app
        .create_notification_channel(&user, "Ch".into(), "webhook".into(), "{}".into(), true)
        .await
        .unwrap();

    let sub = app
        .create_notification_subscription(
            &user,
            ch.id,
            "release_grabbed".into(),
            "global".into(),
            None,
            true,
        )
        .await
        .unwrap();

    let err = app
        .update_notification_subscription(
            &user,
            sub.id,
            Some("bogus_event".into()),
            None,
            NotificationScopeIdUpdate::NoChange,
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
}

#[tokio::test]
async fn create_subscription_rejects_unsubscribable_event_type() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let channel = app
        .create_notification_channel(&user, "Ch".into(), "webhook".into(), "{}".into(), true)
        .await
        .unwrap();

    let err = app
        .create_notification_subscription(
            &user,
            channel.id,
            "test".into(),
            "global".into(),
            None,
            true,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
}

#[tokio::test]
async fn notification_event_types_query_returns_only_dispatchable_subscription_events() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);

    let body = schema_exec(
        &app,
        &ctx,
        r#"
        query NotificationEventTypes {
          notificationEventTypes
        }
        "#,
    )
    .await;

    assert_no_errors(&body);
    let event_types = body["data"]["notificationEventTypes"]
        .as_array()
        .expect("event type array")
        .iter()
        .map(|value| value.as_str().expect("event type string"))
        .collect::<Vec<_>>();

    assert_eq!(
        event_types,
        vec![
            "title_added",
            "title_deleted",
            "grab",
            "download",
            "import_complete",
            "import_rejected",
            "upgrade",
            "rename",
            "file_deleted_for_upgrade",
            "file_deleted",
            "post_processing_completed",
            "subtitle_downloaded",
            "subtitle_search_failed",
            "media_request_submitted",
            "media_request_approved",
            "media_request_rejected",
            "media_request_canceled",
        ]
    );
}

#[tokio::test]
async fn notification_provider_types_query_exposes_jellyfin_multiline_field() {
    let ctx = TestContext::new().await;
    let provider = Arc::new(FakeNotificationProvider::jellyfin());
    let app = app_with_notification_provider(&ctx, provider);

    let body = schema_exec(
        &app,
        &ctx,
        r#"
        query NotificationProviderTypes {
          notificationProviderTypes {
            providerType
            name
            supportedEvents
            supportsTest
            configFields {
              key
              fieldType
              required
            }
          }
        }
        "#,
    )
    .await;

    assert_no_errors(&body);
    let providers = body["data"]["notificationProviderTypes"]
        .as_array()
        .expect("provider array");
    let jellyfin = providers
        .iter()
        .find(|provider| provider["providerType"] == "jellyfin")
        .expect("jellyfin provider");

    assert_eq!(jellyfin["name"], "Jellyfin");
    assert_eq!(
        jellyfin["supportedEvents"],
        json!(vec![
            "import_complete",
            "upgrade",
            "rename",
            "file_deleted",
            "file_deleted_for_upgrade",
        ]),
    );
    assert_eq!(jellyfin["supportsTest"], true);
    assert!(
        jellyfin["configFields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| {
                field["key"] == "path_mappings"
                    && field["fieldType"] == "MULTILINE"
                    && field["required"] == false
            }),
        "expected path_mappings multiline field in {jellyfin}"
    );
}

#[tokio::test]
async fn jellyfin_media_server_connection_is_notification_target() {
    let ctx = TestContext::new().await;
    let app = app_with_notification_provider(&ctx, Arc::new(FakeNotificationProvider::jellyfin()));
    let user = default_user(&app).await;

    let connection = insert_jellyfin_media_server_connection(
        &ctx,
        "jellyfin-notification-target",
        "http://jellyfin:8096",
        jellyfin_path_mappings(),
    )
    .await;

    let targets = app
        .list_notification_targets(&user)
        .await
        .expect("list notification targets");
    let target = targets
        .iter()
        .find(|target| target.id == connection.id)
        .expect("media server target should be listed");
    assert_eq!(target.target_kind.as_str(), "media_server_connection");
    assert_eq!(target.provider_type, "jellyfin");
    assert_eq!(
        target.media_server_connection_id.as_deref(),
        Some(connection.id.as_str())
    );

    let subscription = app
        .create_notification_subscription_for_target(
            &user,
            NotificationSubscriptionTargetCreate {
                channel_id: None,
                target_kind: Some("media_server_connection".into()),
                target_id: Some(connection.id.clone()),
                event_type: NotificationEventType::ImportComplete.as_str().to_string(),
                scope: "global".into(),
                scope_id: None,
                is_enabled: true,
            },
        )
        .await
        .expect("create media server target subscription");
    assert_eq!(subscription.channel_id, None);
    assert_eq!(subscription.target_kind.as_str(), "media_server_connection");
    assert_eq!(subscription.target_id, connection.id);
}

#[tokio::test]
async fn media_server_connections_are_listed_without_notification_plugin_or_api_key() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let connection = insert_jellyfin_media_server_connection_with_api_key(
        &ctx,
        "jellyfin-visible-target",
        "http://jellyfin:8096",
        None,
        jellyfin_path_mappings(),
    )
    .await;

    let targets = app
        .list_notification_targets(&user)
        .await
        .expect("list notification targets");

    let target = targets
        .iter()
        .find(|target| target.id == connection.id)
        .expect("media server connection should be visible as a notification target");
    assert_eq!(target.target_kind.as_str(), "media_server_connection");
    assert_eq!(target.provider_type, "jellyfin");
    assert_eq!(target.is_enabled, connection.enabled);
}

#[tokio::test]
async fn jellyfin_dist_plugin_accepts_test_notification_payload() {
    let Some(provider) = load_jellyfin_dist_provider() else {
        return;
    };
    let ctx = TestContext::new().await;

    Mock::given(method("GET"))
        .and(path("/System/Info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ServerName": "Jellyfin Test",
            "Version": "10.10.0",
        })))
        .expect(1)
        .mount(&ctx.nzbgeek_server)
        .await;

    let channel = jellyfin_channel_config(&ctx.nzbgeek_server.uri(), "/data => /mnt");
    let client = provider
        .client_for_channel(&channel)
        .expect("jellyfin client should load");

    client
        .send_notification(&test_notification_payload())
        .await
        .expect("jellyfin dist plugin should accept test payload");
}

#[tokio::test]
async fn notification_dispatcher_delivers_jellyfin_media_server_target_refresh() {
    let Some(provider) = load_jellyfin_dist_provider() else {
        return;
    };
    let ctx = TestContext::new().await;
    let app = app_with_notification_provider(&ctx, provider);
    let user = default_user(&app).await;

    Mock::given(method("POST"))
        .and(path("/Library/Media/Updated"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&ctx.nzbgeek_server)
        .await;

    let connection = insert_jellyfin_media_server_connection(
        &ctx,
        "jellyfin-dispatch-target",
        &ctx.nzbgeek_server.uri(),
        vec![MediaServerPathMapping {
            source_path: "/media".to_string(),
            destination_path: "/data".to_string(),
            sort_order: 0,
        }],
    )
    .await;
    create_media_server_subscription(
        &app,
        &user,
        &connection,
        NotificationEventType::ImportComplete.as_str(),
    )
    .await;

    let cancel = CancellationToken::new();
    let dispatcher = tokio::spawn(start_notification_dispatcher(app.clone(), cancel.clone()));
    tokio::task::yield_now().await;

    app.append_domain_event(new_event(
        "evt-jellyfin-media-server-target-refresh",
        "title-1",
        "series",
        DomainEventPayload::ImportCompleted(import_completed_event_data(
            title_context(
                "Example Show",
                "series",
                DomainExternalIds {
                    imdb_id: None,
                    tmdb_id: None,
                    tvdb_id: Some("123".to_string()),
                    anidb_id: None,
                },
            ),
            vec![MediaPathUpdate {
                path: "/data/series/Example Show/S01E01.mkv".to_string(),
                update_type: MediaUpdateType::Created,
            }],
            1,
            vec!["episode-1".to_string()],
        )),
    ))
    .await
    .expect("append import-complete event");

    let requests = wait_for_wiremock_requests(&ctx.nzbgeek_server, 1).await;
    cancel.cancel();
    dispatcher.await.expect("dispatcher task");

    assert_eq!(requests[0].url.path(), "/Library/Media/Updated");
    let body: Value = serde_json::from_slice(&requests[0].body).expect("json body");
    assert_eq!(
        body,
        json!({
            "updates": [
                {
                    "path": "/media/series/Example Show/S01E01.mkv",
                    "updateType": "Created",
                }
            ]
        }),
    );
}

#[tokio::test]
async fn jellyfin_dist_plugin_refreshes_mapped_media_file_for_import_complete() {
    let Some(provider) = load_jellyfin_dist_provider() else {
        return;
    };
    let ctx = TestContext::new().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&ctx.nzbgeek_server)
        .await;

    let channel = jellyfin_channel_config(
        &ctx.nzbgeek_server.uri(),
        "/data/Movies => /mnt/media/Movies\n/data/TV => /mnt/media/TV",
    );
    let client = provider
        .client_for_channel(&channel)
        .expect("jellyfin client should load");

    let title = jellyfin_title_payload(
        "Example Movie",
        "movie",
        Some("/data/Movies/Example Movie (2024)"),
        NotificationExternalIdsPayload {
            tmdb_id: Some("987".to_string()),
            imdb_id: Some("tt6543210".to_string()),
            ..Default::default()
        },
    );
    let payload = jellyfin_notification_payload(
        NotificationEventType::ImportComplete,
        Some(title),
        Some(NotificationFilePayload {
            primary_path: Some("/data/Movies/Example Movie (2024)/Example Movie.mkv".to_string()),
            media_updates: vec![NotificationMediaUpdatePayload {
                path: "/data/Movies/Example Movie (2024)/Example Movie.mkv".to_string(),
                update_type: NotificationMediaUpdateTypePayload::Created,
            }],
        }),
        vec![NotificationMediaFilePayload {
            id: Some("file-1".to_string()),
            path: "/data/Movies/Example Movie (2024)/Example Movie.mkv".to_string(),
            ..Default::default()
        }],
    );

    client
        .send_notification(&payload)
        .await
        .expect("import_complete should refresh Jellyfin");

    let requests = ctx
        .nzbgeek_server
        .received_requests()
        .await
        .expect("request capture should succeed");
    assert_eq!(requests.len(), 1, "expected one mapped refresh request");
    assert_eq!(requests[0].url.path(), "/Library/Media/Updated");

    let body: Value = serde_json::from_slice(&requests[0].body).expect("json body");
    assert_eq!(
        body,
        json!({
            "updates": [
                {
                    "path": "/mnt/media/Movies/Example Movie (2024)/Example Movie.mkv",
                    "updateType": "Created",
                }
            ]
        }),
    );
}

#[tokio::test]
async fn jellyfin_dist_plugin_refreshes_rename_once_with_modified_update() {
    let Some(provider) = load_jellyfin_dist_provider() else {
        return;
    };
    let ctx = TestContext::new().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&ctx.nzbgeek_server)
        .await;

    let channel = jellyfin_channel_config(
        &ctx.nzbgeek_server.uri(),
        "/data/Movies => /mnt/media/Movies\n/data/TV => /mnt/media/TV",
    );
    let client = provider
        .client_for_channel(&channel)
        .expect("jellyfin client should load");

    let payload = jellyfin_notification_payload(
        NotificationEventType::Rename,
        Some(jellyfin_title_payload(
            "Example Show",
            "series",
            Some("/data/TV/Example Show"),
            NotificationExternalIdsPayload {
                tvdb_id: Some("123".to_string()),
                imdb_id: Some("tt456".to_string()),
                ..Default::default()
            },
        )),
        Some(NotificationFilePayload {
            primary_path: Some("/data/TV/Example Show/New Name.mkv".to_string()),
            media_updates: vec![
                NotificationMediaUpdatePayload {
                    path: "/data/TV/Example Show/Old Name.mkv".to_string(),
                    update_type: NotificationMediaUpdateTypePayload::Deleted,
                },
                NotificationMediaUpdatePayload {
                    path: "/data/TV/Example Show/New Name.mkv".to_string(),
                    update_type: NotificationMediaUpdateTypePayload::Created,
                },
            ],
        }),
        vec![NotificationMediaFilePayload {
            id: Some("file-episode-1".to_string()),
            path: "/data/TV/Example Show/New Name.mkv".to_string(),
            ..Default::default()
        }],
    );

    client
        .send_notification(&payload)
        .await
        .expect("rename should refresh Jellyfin");

    let requests = ctx
        .nzbgeek_server
        .received_requests()
        .await
        .expect("request capture should succeed");
    assert_eq!(requests.len(), 1, "expected one rename refresh request");
    assert_eq!(requests[0].url.path(), "/Library/Media/Updated");

    let body: Value = serde_json::from_slice(&requests[0].body).expect("json body");
    assert_eq!(
        body,
        json!({
            "updates": [
                {
                    "path": "/mnt/media/TV/Example Show/Old Name.mkv",
                    "updateType": "Deleted",
                },
                {
                    "path": "/mnt/media/TV/Example Show/New Name.mkv",
                    "updateType": "Created",
                }
            ]
        }),
    );
}

#[tokio::test]
async fn jellyfin_dist_plugin_refreshes_file_deleted_once_with_deleted_update() {
    let Some(provider) = load_jellyfin_dist_provider() else {
        return;
    };
    let ctx = TestContext::new().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&ctx.nzbgeek_server)
        .await;

    let channel = jellyfin_channel_config(
        &ctx.nzbgeek_server.uri(),
        "/data/Movies => /mnt/media/Movies\n/data/TV => /mnt/media/TV",
    );
    let client = provider
        .client_for_channel(&channel)
        .expect("jellyfin client should load");

    let payload = jellyfin_notification_payload(
        NotificationEventType::FileDeleted,
        Some(jellyfin_title_payload(
            "Example Movie",
            "movie",
            Some("/data/Movies/Example Movie (2024)"),
            NotificationExternalIdsPayload {
                tmdb_id: Some("987".to_string()),
                imdb_id: Some("tt6543210".to_string()),
                ..Default::default()
            },
        )),
        Some(NotificationFilePayload {
            primary_path: Some("/data/Movies/Example Movie (2024)/Example Movie.mkv".to_string()),
            media_updates: vec![NotificationMediaUpdatePayload {
                path: "/data/Movies/Example Movie (2024)/Example Movie.mkv".to_string(),
                update_type: NotificationMediaUpdateTypePayload::Deleted,
            }],
        }),
        Vec::new(),
    );

    client
        .send_notification(&payload)
        .await
        .expect("file_deleted should refresh Jellyfin");

    let requests = ctx
        .nzbgeek_server
        .received_requests()
        .await
        .expect("request capture should succeed");
    assert_eq!(requests.len(), 1, "expected one delete refresh request");
    assert_eq!(requests[0].url.path(), "/Library/Media/Updated");

    let body: Value = serde_json::from_slice(&requests[0].body).expect("json body");
    assert_eq!(
        body,
        json!({
            "updates": [
                {
                    "path": "/mnt/media/Movies/Example Movie (2024)/Example Movie.mkv",
                    "updateType": "Deleted",
                }
            ]
        }),
    );
}

#[tokio::test]
async fn jellyfin_dist_plugin_falls_back_to_movie_and_series_id_updates_when_paths_do_not_map() {
    let Some(provider) = load_jellyfin_dist_provider() else {
        return;
    };
    let ctx = TestContext::new().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&ctx.nzbgeek_server)
        .await;

    let channel = jellyfin_channel_config(
        &ctx.nzbgeek_server.uri(),
        "/data/Movies => /mnt/media/Movies\n/data/TV => /mnt/media/TV",
    );
    let client = provider
        .client_for_channel(&channel)
        .expect("jellyfin client should load");

    let movie_payload = jellyfin_notification_payload(
        NotificationEventType::ImportComplete,
        Some(jellyfin_title_payload(
            "Fallback Movie",
            "movie",
            None,
            NotificationExternalIdsPayload {
                tmdb_id: Some("987".to_string()),
                imdb_id: Some("tt6543210".to_string()),
                ..Default::default()
            },
        )),
        Some(NotificationFilePayload {
            primary_path: Some("/srv/movies/Fallback Movie/Fallback Movie.mkv".to_string()),
            media_updates: vec![NotificationMediaUpdatePayload {
                path: "/srv/movies/Fallback Movie/Fallback Movie.mkv".to_string(),
                update_type: NotificationMediaUpdateTypePayload::Created,
            }],
        }),
        Vec::new(),
    );
    client
        .send_notification(&movie_payload)
        .await
        .expect("movie fallback should succeed");

    let series_payload = jellyfin_notification_payload(
        NotificationEventType::ImportComplete,
        Some(jellyfin_title_payload(
            "Fallback Show",
            "series",
            None,
            NotificationExternalIdsPayload {
                tvdb_id: Some("123".to_string()),
                ..Default::default()
            },
        )),
        Some(NotificationFilePayload {
            primary_path: Some("/srv/tv/Fallback Show/S01E01.mkv".to_string()),
            media_updates: vec![NotificationMediaUpdatePayload {
                path: "/srv/tv/Fallback Show/S01E01.mkv".to_string(),
                update_type: NotificationMediaUpdateTypePayload::Created,
            }],
        }),
        Vec::new(),
    );
    client
        .send_notification(&series_payload)
        .await
        .expect("series fallback should succeed");

    let requests = ctx
        .nzbgeek_server
        .received_requests()
        .await
        .expect("request capture should succeed");
    assert_eq!(
        requests.len(),
        2,
        "expected one movie and one series fallback"
    );

    let movie_request = requests
        .iter()
        .find(|request| request.url.path() == "/Library/Movies/Updated")
        .expect("movie fallback request");
    let movie_query = movie_request
        .url
        .query_pairs()
        .into_owned()
        .collect::<HashMap<_, _>>();
    assert_eq!(movie_query.get("tmdbId").map(String::as_str), Some("987"));
    assert_eq!(
        movie_query.get("imdbId").map(String::as_str),
        Some("tt6543210"),
    );

    let series_request = requests
        .iter()
        .find(|request| request.url.path() == "/Library/Series/Updated")
        .expect("series fallback request");
    let series_query = series_request
        .url
        .query_pairs()
        .into_owned()
        .collect::<HashMap<_, _>>();
    assert_eq!(series_query.get("tvdbId").map(String::as_str), Some("123"));
}

#[tokio::test]
async fn jellyfin_dist_plugin_requires_media_updates_for_non_test_events() {
    let Some(provider) = load_jellyfin_dist_provider() else {
        return;
    };
    let ctx = TestContext::new().await;

    let channel = jellyfin_channel_config(
        &ctx.nzbgeek_server.uri(),
        "/data/Movies => /mnt/media/Movies\n/data/TV => /mnt/media/TV",
    );
    let client = provider
        .client_for_channel(&channel)
        .expect("jellyfin client should load");

    let payload = jellyfin_notification_payload(
        NotificationEventType::Download,
        Some(jellyfin_title_payload(
            "Unsupported Download",
            "movie",
            Some("/data/Movies/Unsupported Download"),
            NotificationExternalIdsPayload::default(),
        )),
        None,
        Vec::new(),
    );

    let err = client
        .send_notification(&payload)
        .await
        .expect_err("non-test Jellyfin notification without media updates should fail");
    assert!(
        err.to_string().contains("file.media_updates is required"),
        "unexpected Jellyfin error: {err:?}"
    );

    let requests = ctx
        .nzbgeek_server
        .received_requests()
        .await
        .expect("request capture should succeed");
    assert!(
        requests.is_empty(),
        "invalid Jellyfin notification should not make HTTP requests"
    );
}

#[tokio::test]
async fn notification_dispatcher_delivers_global_media_request_to_facet_scope() {
    let ctx = TestContext::new().await;
    let provider = Arc::new(FakeNotificationProvider::webhook());
    let app = app_with_notification_provider(&ctx, provider.clone());
    let user = default_user(&app).await;
    let channel = app
        .create_notification_channel(&user, "Webhook".into(), "webhook".into(), "{}".into(), true)
        .await
        .expect("channel should be created");
    app.create_notification_subscription(
        &user,
        channel.id,
        NotificationEventType::MediaRequestSubmitted
            .as_str()
            .to_string(),
        "facet".into(),
        Some("series".into()),
        true,
    )
    .await
    .expect("subscription should be created");

    let cancel = CancellationToken::new();
    let dispatcher = tokio::spawn(start_notification_dispatcher(app.clone(), cancel.clone()));
    app.append_domain_event(NewDomainEvent {
        event_id: "evt-media-request-facet-scope".to_string(),
        occurred_at: Utc::now(),
        actor_kind: DomainEventActorKind::User,
        actor_user_id: Some("requester-1".to_string()),
        actor_display_name: "requester-1".to_string(),
        title_id: None,
        facet: None,
        correlation_id: None,
        causation_id: None,
        schema_version: 1,
        stream: DomainEventStream::Global,
        payload: DomainEventPayload::MediaRequestSubmitted(MediaRequestSubmittedEventData {
            requested_lease_days: None,
            request_id: "request-1".to_string(),
            library_id: "library-series".to_string(),
            facet: MediaFacet::Series,
            title_name: "Requested Show".to_string(),
            external_ids: Vec::new(),
            poster_url: None,
            year: None,
            requested_quality_profile_id: None,
            requested_quality_profile_name: None,
            requested_monitor_type: None,
        }),
    })
    .await
    .expect("media request event should append");

    let captured = wait_for_captured(&provider, 1).await;
    assert_eq!(
        captured[0].event_type,
        NotificationEventType::MediaRequestSubmitted.as_str()
    );
    cancel.cancel();
    dispatcher.await.expect("dispatcher should stop");
}

#[tokio::test]
async fn notification_dispatcher_deduplicates_overlapping_file_delete_subscriptions() {
    let ctx = TestContext::new().await;
    let provider = Arc::new(FakeNotificationProvider::webhook());
    let app = app_with_notification_provider(&ctx, provider.clone());
    let user = default_user(&app).await;
    let channel = app
        .create_notification_channel(&user, "Webhook".into(), "webhook".into(), "{}".into(), true)
        .await
        .expect("channel should be created");
    for event_type in [
        NotificationEventType::FileDeleted,
        NotificationEventType::FileDeletedForUpgrade,
    ] {
        app.create_notification_subscription(
            &user,
            channel.id.clone(),
            event_type.as_str().to_string(),
            "global".into(),
            None,
            true,
        )
        .await
        .expect("subscription should be created");
    }

    let cancel = CancellationToken::new();
    let dispatcher = tokio::spawn(start_notification_dispatcher(app.clone(), cancel.clone()));
    app.append_domain_event(new_event(
        "evt-file-delete-deduplicated",
        "title-1",
        "movie",
        DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData {
            title: title_context("Upgrade Movie", "movie", DomainExternalIds::default()),
            media_updates: vec![MediaPathUpdate {
                path: "/library/Upgrade Movie.old.mkv".to_string(),
                update_type: MediaUpdateType::Deleted,
            }],
            file_id: Some("file-old".to_string()),
            reason: MediaFileDeletedReason::UpgradeCleanup,
            episode_ids: Vec::new(),
        }),
    ))
    .await
    .expect("file deletion event should append");

    wait_for_captured(&provider, 1).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(provider.captured().len(), 1);
    cancel.cancel();
    dispatcher.await.expect("dispatcher should stop");
}

#[tokio::test]
async fn notification_dispatcher_delivers_structured_lifecycle_metadata() {
    let ctx = TestContext::new().await;
    let provider = Arc::new(FakeNotificationProvider::jellyfin());
    let app = app_with_notification_provider(&ctx, provider.clone());
    let user = default_user(&app).await;

    let connection = insert_jellyfin_media_server_connection(
        &ctx,
        "jellyfin-lifecycle-target",
        "http://jellyfin:8096",
        jellyfin_path_mappings(),
    )
    .await;

    for event_type in [
        DomainEventType::ImportCompleted,
        DomainEventType::MediaFileUpgraded,
        DomainEventType::MediaFileRenamed,
        DomainEventType::MediaFileDeleted,
    ] {
        create_media_server_subscription(&app, &user, &connection, event_type.as_str()).await;
    }

    let cancel = CancellationToken::new();
    let dispatcher = tokio::spawn(start_notification_dispatcher(app.clone(), cancel.clone()));
    tokio::task::yield_now().await;

    let scenarios = vec![
        (
            "import_complete",
            "Import complete: Example Show".to_string(),
            "Imported 1 file for 'Example Show'.".to_string(),
            lifecycle_metadata(
                "Example Show",
                "series",
                vec![("/data/TV/Example Show/S01E01.mkv", "created")],
                json!({ "tvdb_id": "123", "imdb_id": "tt456" }),
            ),
            new_event(
                "evt-import-complete",
                "title-1",
                "series",
                DomainEventPayload::ImportCompleted(import_completed_event_data(
                    title_context(
                        "Example Show",
                        "series",
                        DomainExternalIds {
                            imdb_id: Some("tt456".to_string()),
                            tmdb_id: None,
                            tvdb_id: Some("123".to_string()),
                            anidb_id: None,
                        },
                    ),
                    vec![MediaPathUpdate {
                        path: "/data/TV/Example Show/S01E01.mkv".to_string(),
                        update_type: MediaUpdateType::Created,
                    }],
                    1,
                    vec!["episode-1".to_string()],
                )),
            ),
        ),
        (
            "upgrade",
            "Upgraded: Example Movie".to_string(),
            "Upgraded file for 'Example Movie'.".to_string(),
            lifecycle_metadata(
                "Example Movie",
                "movie",
                vec![("/data/Movies/Example Movie (2024)/Example Movie.mkv", "modified")],
                json!({ "tmdb_id": "987", "imdb_id": "tt6543210" }),
            ),
            new_event(
                "evt-upgrade",
                "title-1",
                "movie",
                DomainEventPayload::MediaFileUpgraded(MediaFileUpgradedEventData {
                    title: title_context(
                        "Example Movie",
                        "movie",
                        DomainExternalIds {
                            imdb_id: Some("tt6543210".to_string()),
                            tmdb_id: Some("987".to_string()),
                            tvdb_id: None,
                            anidb_id: None,
                        },
                    ),
                    media_updates: vec![MediaPathUpdate {
                        path: "/data/Movies/Example Movie (2024)/Example Movie.mkv".to_string(),
                        update_type: MediaUpdateType::Modified,
                    }],
                    episode_ids: Vec::new(),
                    previous_file_id: Some("file-old".to_string()),
                    current_file_id: Some("file-new".to_string()),
                    old_score: None,
                    new_score: None,
                    size_bytes: None,
                }),
            ),
        ),
        (
            "rename",
            "Renamed: Example Show".to_string(),
            "Renamed 1 file(s) for 'Example Show'.".to_string(),
            lifecycle_metadata(
                "Example Show",
                "series",
                vec![
                    ("/data/TV/Example Show/Old Name.mkv", "deleted"),
                    ("/data/TV/Example Show/New Name.mkv", "created"),
                ],
                json!({ "tvdb_id": "123", "imdb_id": "tt456" }),
            ),
            new_event(
                "evt-rename",
                "title-1",
                "series",
                DomainEventPayload::MediaFileRenamed(MediaFileRenamedEventData {
                    title: title_context(
                        "Example Show",
                        "series",
                        DomainExternalIds {
                            imdb_id: Some("tt456".to_string()),
                            tmdb_id: None,
                            tvdb_id: Some("123".to_string()),
                            anidb_id: None,
                        },
                    ),
                    media_updates: vec![
                        MediaPathUpdate {
                            path: "/data/TV/Example Show/Old Name.mkv".to_string(),
                            update_type: MediaUpdateType::Deleted,
                        },
                        MediaPathUpdate {
                            path: "/data/TV/Example Show/New Name.mkv".to_string(),
                            update_type: MediaUpdateType::Created,
                        },
                    ],
                    renamed_count: 1,
                    episode_ids: vec!["episode-1".to_string()],
                }),
            ),
        ),
        (
            "file_deleted",
            "File deleted: Example Movie".to_string(),
            "Deleted media file from disk: /data/Movies/Example Movie (2024)/Example Movie.mkv"
                .to_string(),
            lifecycle_metadata(
                "Example Movie",
                "movie",
                vec![("/data/Movies/Example Movie (2024)/Example Movie.mkv", "deleted")],
                json!({ "tmdb_id": "987", "imdb_id": "tt6543210" }),
            ),
            new_event(
                "evt-file-deleted",
                "title-1",
                "movie",
                DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData {
                    title: title_context(
                        "Example Movie",
                        "movie",
                        DomainExternalIds {
                            imdb_id: Some("tt6543210".to_string()),
                            tmdb_id: Some("987".to_string()),
                            tvdb_id: None,
                            anidb_id: None,
                        },
                    ),
                    media_updates: vec![MediaPathUpdate {
                        path: "/data/Movies/Example Movie (2024)/Example Movie.mkv".to_string(),
                        update_type: MediaUpdateType::Deleted,
                    }],
                    file_id: Some("file-1".to_string()),
                    reason: MediaFileDeletedReason::Deleted,
                    episode_ids: Vec::new(),
                }),
            ),
        ),
        (
            "file_deleted_for_upgrade",
            "Deleted for upgrade: Example Movie".to_string(),
            "Removed old media file during upgrade: /data/Movies/Example Movie (2024)/Example Movie.old.mkv"
                .to_string(),
            lifecycle_metadata(
                "Example Movie",
                "movie",
                vec![(
                    "/data/Movies/Example Movie (2024)/Example Movie.old.mkv",
                    "deleted",
                )],
                json!({ "tmdb_id": "987", "imdb_id": "tt6543210" }),
            ),
            new_event(
                "evt-file-deleted-upgrade",
                "title-1",
                "movie",
                DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData {
                    title: title_context(
                        "Example Movie",
                        "movie",
                        DomainExternalIds {
                            imdb_id: Some("tt6543210".to_string()),
                            tmdb_id: Some("987".to_string()),
                            tvdb_id: None,
                            anidb_id: None,
                        },
                    ),
                    media_updates: vec![MediaPathUpdate {
                        path: "/data/Movies/Example Movie (2024)/Example Movie.old.mkv"
                            .to_string(),
                        update_type: MediaUpdateType::Deleted,
                    }],
                    file_id: Some("file-old".to_string()),
                    reason: MediaFileDeletedReason::UpgradeCleanup,
                    episode_ids: Vec::new(),
                }),
            ),
        ),
    ];

    for (_plugin_event_type, _title, _body, _metadata, event) in &scenarios {
        app.append_domain_event(event.clone())
            .await
            .expect("append domain event");
    }

    let captured = wait_for_captured(&provider, scenarios.len()).await;
    cancel.cancel();
    dispatcher.await.expect("dispatcher task");

    let expected = scenarios
        .into_iter()
        .map(
            |(event_type, title, body, metadata, _event)| CapturedNotification {
                event_type: event_type.to_string(),
                title,
                message: body,
                metadata,
            },
        )
        .collect::<Vec<_>>();

    assert_eq!(captured, expected);
}

#[tokio::test]
async fn notification_dispatcher_prefers_local_catalog_metadata_over_snapshot() {
    let ctx = TestContext::new().await;
    let provider = Arc::new(FakeNotificationProvider::jellyfin());
    let app = app_with_notification_provider(&ctx, provider.clone());
    let user = default_user(&app).await;
    // Creation is registry-gated now, so the label this fixture asserts on has
    // to exist before a title can be born carrying it.
    app.create_title_tag_definition(&user, "local-tag", None)
        .await
        .expect("tag should be defined");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Canonical Show".to_string(),
                facet: MediaFacet::Series,
                monitored: true,
                year: Some(2024),
                tags: vec!["local-tag".to_string()],
                external_ids: vec![
                    external_id("tvdb", "321"),
                    external_id("imdb", "tt7654321"),
                    external_id("anilist", "9999"),
                ],
                ..Default::default()
            },
        )
        .await
        .expect("add title");

    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season 1".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");

    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(collection.id.clone()),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("1".into()),
            Some("Pilot".into()),
            Some("2024-01-01".into()),
            Some(1500),
            false,
            true,
        )
        .await
        .expect("create episode");

    let connection = insert_jellyfin_media_server_connection(
        &ctx,
        "jellyfin-local-catalog-target",
        "http://jellyfin:8096",
        jellyfin_path_mappings(),
    )
    .await;
    create_media_server_subscription(
        &app,
        &user,
        &connection,
        DomainEventType::ImportCompleted.as_str(),
    )
    .await;

    let cancel = CancellationToken::new();
    let dispatcher = tokio::spawn(start_notification_dispatcher(app.clone(), cancel.clone()));
    tokio::task::yield_now().await;

    app.append_domain_event(new_event(
        "evt-local-enrichment",
        &title.id,
        "series",
        DomainEventPayload::ImportCompleted(import_completed_event_data(
            title_context(
                "Snapshot Show",
                "series",
                DomainExternalIds {
                    imdb_id: Some("tt0000000".to_string()),
                    tmdb_id: None,
                    tvdb_id: Some("999".to_string()),
                    anidb_id: None,
                },
            ),
            vec![MediaPathUpdate {
                path: "/data/TV/Canonical Show/S01E01.mkv".to_string(),
                update_type: MediaUpdateType::Created,
            }],
            1,
            vec![episode.id.clone()],
        )),
    ))
    .await
    .expect("append domain event");

    let captured = wait_for_captured(&provider, 1).await;
    cancel.cancel();
    dispatcher.await.expect("dispatcher task");

    let metadata = &captured[0].metadata;
    assert_eq!(metadata.get("title_name"), Some(&json!("Canonical Show")));
    assert_eq!(metadata.get("title_year"), Some(&json!(2024)));
    assert_eq!(
        metadata.get("title_tags"),
        Some(&json!(vec!["local-tag".to_string()]))
    );
    assert_eq!(
        metadata.get("external_ids"),
        Some(&json!({
            "tvdb_id": "321",
            "imdb_id": "tt7654321",
        }))
    );
    assert_eq!(
        metadata.get("external_ids_by_source"),
        Some(&json!({
            "anilist": ["9999"],
            "imdb": ["tt7654321"],
            "tvdb": ["321"],
        }))
    );
    assert_eq!(metadata.get("episode_id"), Some(&json!(episode.id)));
    assert_eq!(metadata.get("episode_season_number"), Some(&json!("1")));
    assert_eq!(metadata.get("episode_number"), Some(&json!("1")));
    assert_eq!(metadata.get("episode_title"), Some(&json!("Pilot")));
    assert_eq!(metadata.get("episode_air_date"), Some(&json!("2024-01-01")));
}

#[tokio::test]
async fn notification_dispatcher_replays_notifications_after_operational_burst() {
    let ctx = TestContext::new().await;
    let provider = Arc::new(FakeNotificationProvider::jellyfin());
    let app = app_with_notification_provider(&ctx, provider.clone());
    let user = default_user(&app).await;

    let connection = insert_jellyfin_media_server_connection(
        &ctx,
        "jellyfin-replay-target",
        "http://jellyfin:8096",
        jellyfin_path_mappings(),
    )
    .await;
    create_media_server_subscription(
        &app,
        &user,
        &connection,
        DomainEventType::ImportCompleted.as_str(),
    )
    .await;

    for i in 0..300 {
        app.append_domain_event(new_event(
            &format!("evt-scan-{i}"),
            "title-scan",
            "movie",
            DomainEventPayload::LibraryScanProgressed(LibraryScanProgressedEventData {
                session_id: format!("scan-{i}"),
                status: "running".to_string(),
                found_titles: i as i64 + 1,
                title_match_completed: 0,
                title_match_total_known: false,
                titles_completed: i as i64 + 1,
                titles_total: Some(300),
                files_completed: i as i64 + 1,
                files_total: Some(300),
                warning_message: None,
            }),
        ))
        .await
        .expect("operational burst event should append");
    }

    let cancel = CancellationToken::new();
    let dispatcher = tokio::spawn(start_notification_dispatcher(app.clone(), cancel.clone()));
    tokio::task::yield_now().await;

    app.append_domain_event(new_event(
        "evt-import-after-burst",
        "title-1",
        "series",
        DomainEventPayload::ImportCompleted(import_completed_event_data(
            title_context(
                "Burst Replay Show",
                "series",
                DomainExternalIds {
                    imdb_id: Some("tt456".to_string()),
                    tmdb_id: None,
                    tvdb_id: Some("123".to_string()),
                    anidb_id: None,
                },
            ),
            vec![MediaPathUpdate {
                path: "/data/TV/Burst Replay Show/S01E01.mkv".to_string(),
                update_type: MediaUpdateType::Created,
            }],
            1,
            vec!["episode-1".to_string()],
        )),
    ))
    .await
    .expect("notification event should append");

    let captured = wait_for_captured(&provider, 1).await;
    cancel.cancel();
    dispatcher.await.expect("dispatcher task");

    assert_eq!(captured.len(), 1);
    assert_eq!(
        captured[0].event_type,
        NotificationEventType::ImportComplete.as_str()
    );
    assert_eq!(captured[0].title, "Import complete: Burst Replay Show");
    assert_eq!(
        captured[0].message,
        "Imported 1 file for 'Burst Replay Show'."
    );
}

#[tokio::test]
async fn notification_dispatcher_ignores_operational_burst_while_running() {
    let ctx = TestContext::new().await;
    let provider = Arc::new(FakeNotificationProvider::jellyfin());
    let app = app_with_notification_provider(&ctx, provider.clone());
    let user = default_user(&app).await;

    let connection = insert_jellyfin_media_server_connection(
        &ctx,
        "jellyfin-live-burst-target",
        "http://jellyfin:8096",
        jellyfin_path_mappings(),
    )
    .await;
    create_media_server_subscription(
        &app,
        &user,
        &connection,
        DomainEventType::ImportCompleted.as_str(),
    )
    .await;

    let mut wake_rx = app.notification_wake_receiver();
    let cancel = CancellationToken::new();
    let dispatcher = tokio::spawn(start_notification_dispatcher(app.clone(), cancel.clone()));
    tokio::task::yield_now().await;

    for i in 0..300 {
        app.append_domain_event(new_event(
            &format!("evt-live-scan-{i}"),
            "title-scan",
            "movie",
            DomainEventPayload::LibraryScanProgressed(LibraryScanProgressedEventData {
                session_id: format!("scan-live-{i}"),
                status: "running".to_string(),
                found_titles: i as i64 + 1,
                title_match_completed: 0,
                title_match_total_known: false,
                titles_completed: i as i64 + 1,
                titles_total: Some(300),
                files_completed: i as i64 + 1,
                files_total: Some(300),
                warning_message: None,
            }),
        ))
        .await
        .expect("operational burst event should append");
    }

    assert!(
        matches!(wake_rx.try_recv(), Err(TryRecvError::Empty)),
        "operational bursts should not enqueue notification dispatcher wakes"
    );

    let notification_event = app
        .append_domain_event(new_event(
            "evt-live-import-after-burst",
            "title-1",
            "series",
            DomainEventPayload::ImportCompleted(import_completed_event_data(
                title_context(
                    "Live Burst Show",
                    "series",
                    DomainExternalIds {
                        imdb_id: Some("tt456".to_string()),
                        tmdb_id: None,
                        tvdb_id: Some("123".to_string()),
                        anidb_id: None,
                    },
                ),
                vec![MediaPathUpdate {
                    path: "/data/TV/Live Burst Show/S01E01.mkv".to_string(),
                    update_type: MediaUpdateType::Created,
                }],
                1,
                vec!["episode-1".to_string()],
            )),
        ))
        .await
        .expect("notification event should append");

    let wake = tokio::time::timeout(Duration::from_secs(1), wake_rx.recv())
        .await
        .expect("notification wake should arrive")
        .expect("notification wake channel should stay open");
    assert_eq!(wake, notification_event.sequence);
    assert!(
        matches!(wake_rx.try_recv(), Err(TryRecvError::Empty)),
        "notification event should enqueue exactly one wake"
    );

    let captured = wait_for_captured(&provider, 1).await;
    cancel.cancel();
    dispatcher.await.expect("dispatcher task");

    assert_eq!(captured.len(), 1);
    assert_eq!(
        captured[0].event_type,
        NotificationEventType::ImportComplete.as_str()
    );
    assert_eq!(captured[0].title, "Import complete: Live Burst Show");
    assert_eq!(
        captured[0].message,
        "Imported 1 file for 'Live Burst Show'."
    );
}
