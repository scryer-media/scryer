#![recursion_limit = "256"]

mod common;

use async_graphql::Request;
use async_trait::async_trait;
use chrono::Utc;
use common::TestContext;
use scryer_application::{
    AppError, AppResult, NotificationClient, NotificationPluginProvider,
    start_notification_dispatcher,
};
use scryer_domain::{
    ConfigFieldDef, ConfigFieldOption, ConfigFieldType, DomainEventPayload, DomainEventStream,
    DomainEventType, DomainExternalIds, ImportCompletedEventData, MediaFacet,
    MediaFileDeletedEventData, MediaFileDeletedReason, MediaFileRenamedEventData,
    MediaFileUpgradedEventData, MediaPathUpdate, MediaUpdateType, NewDomainEvent,
    NotificationEventType, TitleContextSnapshot,
};
use scryer_interface::build_schema;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Wire notification repos into the test AppUseCase so CRUD methods don't
/// return "not configured".
fn app_with_notifications(ctx: &TestContext) -> scryer_application::AppUseCase {
    let mut app = ctx.app.clone();
    app.services.notification_channels = Some(Arc::new(ctx.db.clone()));
    app.services.notification_subscriptions = Some(Arc::new(ctx.db.clone()));
    app
}

fn app_with_notification_provider(
    ctx: &TestContext,
    provider: Arc<dyn NotificationPluginProvider>,
) -> scryer_application::AppUseCase {
    let mut app = app_with_notifications(ctx);
    app.services.notification_provider = Some(provider);
    app
}

async fn default_user(app: &scryer_application::AppUseCase) -> scryer_domain::User {
    app.find_or_create_default_user().await.unwrap()
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
    async fn send_notification(
        &self,
        event_type: &str,
        title: &str,
        message: &str,
        metadata: &HashMap<String, Value>,
    ) -> AppResult<()> {
        self.captured.lock().unwrap().push(CapturedNotification {
            event_type: event_type.to_string(),
            title: title.to_string(),
            message: message.to_string(),
            metadata: metadata.clone(),
        });
        Ok(())
    }
}

#[derive(Clone)]
struct FakeNotificationProvider {
    provider_type: String,
    provider_name: String,
    config_fields: Vec<ConfigFieldDef>,
    captured: Arc<Mutex<Vec<CapturedNotification>>>,
}

impl FakeNotificationProvider {
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
                    options: vec![],
                    help_text: None,
                },
                ConfigFieldDef {
                    key: "api_key".to_string(),
                    label: "API Key".to_string(),
                    field_type: ConfigFieldType::Password,
                    required: true,
                    default_value: None,
                    options: vec![],
                    help_text: None,
                },
                ConfigFieldDef {
                    key: "path_mappings".to_string(),
                    label: "Path Mappings".to_string(),
                    field_type: ConfigFieldType::Multiline,
                    required: true,
                    default_value: None,
                    options: vec![ConfigFieldOption {
                        value: "/data => /mnt".to_string(),
                        label: "Example".to_string(),
                    }],
                    help_text: Some("One mapping per line.".to_string()),
                },
            ],
            captured: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn captured(&self) -> Vec<CapturedNotification> {
        self.captured.lock().unwrap().clone()
    }
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
}

fn assert_no_errors(body: &Value) {
    assert!(
        body.get("errors").is_none(),
        "unexpected GraphQL errors: {body}"
    );
}

async fn schema_exec(
    app: &scryer_application::AppUseCase,
    ctx: &TestContext,
    query: &str,
) -> Value {
    let schema = build_schema(app.clone(), ctx.db.clone(), false);
    let user = default_user(app).await;
    let response = schema.execute(Request::new(query).data(user)).await;
    serde_json::to_value(&response).expect("serialize GraphQL response")
}

fn config_json_with_path_mappings() -> String {
    serde_json::json!({
        "base_url": "http://jellyfin:8096",
        "api_key": "secret",
        "path_mappings": "/data/Movies => /mnt/media/Movies\n/data/TV => /mnt/media/TV"
    })
    .to_string()
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

fn new_event(
    event_id: &str,
    title_id: &str,
    facet: &str,
    payload: DomainEventPayload,
) -> NewDomainEvent {
    NewDomainEvent {
        event_id: event_id.to_string(),
        occurred_at: Utc::now(),
        actor_user_id: Some("user-1".to_string()),
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
async fn create_channel_accepts_arbitrary_provider_type() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let ch = app
        .create_notification_channel(
            &user,
            "Jellyfin".into(),
            "  Jellyfin  ".into(),
            "{}".into(),
            true,
        )
        .await
        .expect("create channel");

    assert_eq!(ch.channel_type.as_str(), "jellyfin");
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
    let app = app_with_notifications(&ctx);
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

    assert_eq!(sub.channel_id, ch.id);
    assert_eq!(sub.event_type, NotificationEventType::Grab);
    assert!(sub.is_enabled);

    let subs = app.list_notification_subscriptions(&user).await.unwrap();
    assert_eq!(subs.len(), 1);
}

#[tokio::test]
async fn update_subscription() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
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
            None,
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
    let app = app_with_notifications(&ctx);
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
    let app = app_with_notifications(&ctx);
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
            None,
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
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
    assert!(
        jellyfin["configFields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| {
                field["key"] == "path_mappings"
                    && field["fieldType"] == "multiline"
                    && field["required"] == true
            }),
        "expected path_mappings multiline field in {jellyfin}"
    );
}

#[tokio::test]
async fn create_channel_preserves_multiline_jellyfin_config_json() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;
    let config_json = config_json_with_path_mappings();

    let channel = app
        .create_notification_channel(
            &user,
            "Jellyfin".into(),
            "jellyfin".into(),
            config_json.clone(),
            true,
        )
        .await
        .expect("create channel");

    let fetched = app
        .get_notification_channel(&user, &channel.id)
        .await
        .expect("load channel")
        .expect("channel should exist");
    assert_eq!(fetched.config_json, config_json);
}

#[tokio::test]
async fn notification_dispatcher_delivers_structured_lifecycle_metadata() {
    let ctx = TestContext::new().await;
    let provider = Arc::new(FakeNotificationProvider::jellyfin());
    let app = app_with_notification_provider(&ctx, provider.clone());
    let user = default_user(&app).await;

    let channel = app
        .create_notification_channel(
            &user,
            "Jellyfin".into(),
            "jellyfin".into(),
            config_json_with_path_mappings(),
            true,
        )
        .await
        .expect("create channel");

    for event_type in [
        DomainEventType::ImportCompleted,
        DomainEventType::MediaFileUpgraded,
        DomainEventType::MediaFileRenamed,
        DomainEventType::MediaFileDeleted,
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
        .expect("create subscription");
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
                DomainEventPayload::ImportCompleted(ImportCompletedEventData {
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
                    media_updates: vec![MediaPathUpdate {
                        path: "/data/TV/Example Show/S01E01.mkv".to_string(),
                        update_type: MediaUpdateType::Created,
                    }],
                    imported_count: 1,
                    episode_ids: vec!["episode-1".to_string()],
                }),
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
                    previous_file_id: Some("file-old".to_string()),
                    current_file_id: Some("file-new".to_string()),
                    old_score: None,
                    new_score: None,
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
        app.services
            .append_domain_event(event.clone())
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
