mod common;

use std::sync::Arc;
use common::TestContext;
use scryer_application::AppError;

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

async fn default_user(app: &scryer_application::AppUseCase) -> scryer_domain::User {
    app.find_or_create_default_user().await.unwrap()
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
    assert_eq!(ch.channel_type, "webhook");
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
            "grab".into(),
            "global".into(),
            None,
            true,
        )
        .await
        .expect("create subscription");

    assert_eq!(sub.channel_id, ch.id);
    assert_eq!(sub.event_type, "grab");
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
            "grab".into(),
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
            Some("download".into()),
            None,
            None,
            Some(false),
        )
        .await
        .unwrap();

    assert_eq!(updated.event_type, "download");
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
            "grab".into(),
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
            "grab".into(),
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
            "grab".into(),
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
